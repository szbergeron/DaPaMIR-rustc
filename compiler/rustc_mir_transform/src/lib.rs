// tidy-alphabetical-start
#![feature(assert_matches)]
#![feature(box_patterns)]
#![feature(const_type_name)]
#![feature(cow_is_borrowed)]
#![feature(file_buffered)]
#![feature(if_let_guard)]
#![feature(impl_trait_in_assoc_type)]
#![feature(let_chains)]
#![feature(map_try_insert)]
#![feature(never_type)]
#![feature(round_char_boundary)]
#![feature(try_blocks)]
#![feature(yeet_expr)]
#![warn(unreachable_pub)]
// tidy-alphabetical-end

use hir::ConstContext;
use required_consts::RequiredConstsVisitor;
use rustc_const_eval::check_consts::{self, ConstCx};
use rustc_const_eval::util;
use rustc_data_structures::fx::FxIndexSet;
use rustc_data_structures::steal::Steal;
use rustc_hir as hir;
use rustc_hir::def::{CtorKind, DefKind};
use rustc_hir::def_id::LocalDefId;
use rustc_index::IndexVec;
use rustc_middle::mir::{
    AnalysisPhase, Body, CallSource, ClearCrossCrate, ConstOperand, ConstQualifs, LocalDecl,
    MirPhase, Operand, Place, ProjectionElem, Promoted, RuntimePhase, Rvalue, START_BLOCK,
    SourceInfo, Statement, StatementKind, TerminatorKind,
};
use rustc_middle::ty::{self, TyCtxt, TypeVisitableExt};
use rustc_middle::util::Providers;
use rustc_middle::{bug, query, span_bug};
use rustc_span::source_map::Spanned;
use rustc_span::{DUMMY_SP, sym};
use rustc_trait_selection::traits;
use tracing::{debug, trace};

#[macro_use]
mod pass_manager;

use pass_manager::{self as pm, Lint, MirLint, MirPass, WithMinOptLevel};

mod abort_unwinding_calls;
mod add_call_guards;
mod add_moves_for_packed_drops;
mod add_retag;
mod add_subtyping_projections;
mod check_alignment;
mod check_const_item_mutation;
mod check_packed_ref;
mod check_undefined_transmutes;
// This pass is public to allow external drivers to perform MIR cleanup
pub mod cleanup_post_borrowck;
mod copy_prop;
mod coroutine;
mod cost_checker;
mod coverage;
mod cross_crate_inline;
mod ctfe_limit;
mod dataflow_const_prop;
mod dead_store_elimination;
mod deduce_param_attrs;
mod deduplicate_blocks;
mod deref_separator;
mod dest_prop;
pub mod dump_mir;
mod early_otherwise_branch;
mod elaborate_box_derefs;
mod elaborate_drops;
mod errors;
mod ffi_unwind_calls;
mod function_item_references;
mod gvn;
// Made public so that `mir_drops_elaborated_and_const_checked` can be overridden
// by custom rustc drivers, running all the steps by themselves. See #114628.
pub mod inline;
mod instsimplify;
mod jump_threading;
mod known_panics_lint;
mod large_enums;
mod lint;
mod lower_intrinsics;
mod lower_slice_len;
mod match_branches;
mod mentioned_items;
mod multiple_return_terminators;
mod nrvo;
mod post_drop_elaboration;
mod prettify;
mod promote_consts;
mod ref_prop;
mod remove_noop_landing_pads;
mod remove_place_mention;
mod remove_storage_markers;
mod remove_uninit_drops;
mod remove_unneeded_drops;
mod remove_zsts;
mod required_consts;
mod reveal_all;
mod sanity_check;
mod shim;
mod ssa;
// This pass is public to allow external drivers to perform MIR cleanup
mod parmir;
pub mod simplify;
mod simplify_branches;
mod simplify_comparison_integral;
mod single_use_consts;
mod sroa;
mod unreachable_enum_branching;
mod unreachable_prop;
mod validate;

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

pub fn provide(providers: &mut Providers) {
    coverage::query::provide(providers);
    ffi_unwind_calls::provide(providers);
    shim::provide(providers);
    cross_crate_inline::provide(providers);
    providers.queries = query::Providers {
        mir_keys,
        mir_built,
        mir_const_qualif,
        mir_promoted,
        mir_drops_elaborated_and_const_checked,
        mir_for_ctfe,
        mir_coroutine_witnesses: coroutine::mir_coroutine_witnesses,
        optimized_mir,
        is_mir_available,
        is_ctfe_mir_available: is_mir_available,
        mir_callgraph_reachable: inline::cycle::mir_callgraph_reachable,
        mir_inliner_callees: inline::cycle::mir_inliner_callees,
        promoted_mir,
        deduced_param_attrs: deduce_param_attrs::deduced_param_attrs,
        coroutine_by_move_body_def_id: coroutine::coroutine_by_move_body_def_id,
        ..providers.queries
    };
}

fn remap_mir_for_const_eval_select<'tcx>(
    tcx: TyCtxt<'tcx>,
    mut body: Body<'tcx>,
    context: hir::Constness,
) -> Body<'tcx> {
    for bb in body.basic_blocks.as_mut().iter_mut() {
        let terminator = bb.terminator.as_mut().expect("invalid terminator");
        match terminator.kind {
            TerminatorKind::Call {
                func: Operand::Constant(box ConstOperand { ref const_, .. }),
                ref mut args,
                destination,
                target,
                unwind,
                fn_span,
                ..
            } if let ty::FnDef(def_id, _) = *const_.ty().kind()
                && tcx.is_intrinsic(def_id, sym::const_eval_select) =>
            {
                let Ok([tupled_args, called_in_const, called_at_rt]) = take_array(args) else {
                    unreachable!()
                };
                let ty = tupled_args.node.ty(&body.local_decls, tcx);
                let fields = ty.tuple_fields();
                let num_args = fields.len();
                let func =
                    if context == hir::Constness::Const { called_in_const } else { called_at_rt };
                let (method, place): (fn(Place<'tcx>) -> Operand<'tcx>, Place<'tcx>) =
                    match tupled_args.node {
                        Operand::Constant(_) => {
                            // There is no good way of extracting a tuple arg from a constant
                            // (const generic stuff) so we just create a temporary and deconstruct
                            // that.
                            let local = body.local_decls.push(LocalDecl::new(ty, fn_span));
                            bb.statements.push(Statement {
                                source_info: SourceInfo::outermost(fn_span),
                                kind: StatementKind::Assign(Box::new((
                                    local.into(),
                                    Rvalue::Use(tupled_args.node.clone()),
                                ))),
                            });
                            (Operand::Move, local.into())
                        }
                        Operand::Move(place) => (Operand::Move, place),
                        Operand::Copy(place) => (Operand::Copy, place),
                    };
                let place_elems = place.projection;
                let arguments = (0..num_args)
                    .map(|x| {
                        let mut place_elems = place_elems.to_vec();
                        place_elems.push(ProjectionElem::Field(x.into(), fields[x]));
                        let projection = tcx.mk_place_elems(&place_elems);
                        let place = Place { local: place.local, projection };
                        Spanned { node: method(place), span: DUMMY_SP }
                    })
                    .collect();
                terminator.kind = TerminatorKind::Call {
                    func: func.node,
                    args: arguments,
                    destination,
                    target,
                    unwind,
                    call_source: CallSource::Misc,
                    fn_span,
                };
            }
            _ => {}
        }
    }
    body
}

fn take_array<T, const N: usize>(b: &mut Box<[T]>) -> Result<[T; N], Box<[T]>> {
    let b: Box<[T; N]> = std::mem::take(b).try_into()?;
    Ok(*b)
}

fn is_mir_available(tcx: TyCtxt<'_>, def_id: LocalDefId) -> bool {
    tcx.mir_keys(()).contains(&def_id)
}

/// Finds the full set of `DefId`s within the current crate that have
/// MIR associated with them.
fn mir_keys(tcx: TyCtxt<'_>, (): ()) -> FxIndexSet<LocalDefId> {
    // All body-owners have MIR associated with them.
    let mut set: FxIndexSet<_> = tcx.hir().body_owners().collect();

    // Coroutine-closures (e.g. async closures) have an additional by-move MIR
    // body that isn't in the HIR.
    for body_owner in tcx.hir().body_owners() {
        if let DefKind::Closure = tcx.def_kind(body_owner)
            && tcx.needs_coroutine_by_move_body_def_id(body_owner.to_def_id())
        {
            set.insert(tcx.coroutine_by_move_body_def_id(body_owner).expect_local());
        }
    }

    // tuple struct/variant constructors have MIR, but they don't have a BodyId,
    // so we need to build them separately.
    for item in tcx.hir_crate_items(()).free_items() {
        if let DefKind::Struct | DefKind::Enum = tcx.def_kind(item.owner_id) {
            for variant in tcx.adt_def(item.owner_id).variants() {
                if let Some((CtorKind::Fn, ctor_def_id)) = variant.ctor {
                    set.insert(ctor_def_id.expect_local());
                }
            }
        }
    }

    set
}

fn mir_const_qualif(tcx: TyCtxt<'_>, def: LocalDefId) -> ConstQualifs {
    let const_kind = tcx.hir().body_const_context(def);

    // No need to const-check a non-const `fn`.
    match const_kind {
        Some(ConstContext::Const { .. } | ConstContext::Static(_) | ConstContext::ConstFn) => {}
        None => span_bug!(
            tcx.def_span(def),
            "`mir_const_qualif` should only be called on const fns and const items"
        ),
    }

    // N.B., this `borrow()` is guaranteed to be valid (i.e., the value
    // cannot yet be stolen), because `mir_promoted()`, which steals
    // from `mir_built()`, forces this query to execute before
    // performing the steal.
    let body = &tcx.mir_built(def).borrow();

    if body.return_ty().references_error() {
        // It's possible to reach here without an error being emitted (#121103).
        tcx.dcx().span_delayed_bug(body.span, "mir_const_qualif: MIR had errors");
        return Default::default();
    }

    let ccx = check_consts::ConstCx { body, tcx, const_kind, param_env: tcx.param_env(def) };

    let mut validator = check_consts::check::Checker::new(&ccx);
    validator.check_body();

    // We return the qualifs in the return place for every MIR body, even though it is only used
    // when deciding to promote a reference to a `const` for now.
    validator.qualifs_in_return_place()
}

fn mir_built(tcx: TyCtxt<'_>, def: LocalDefId) -> &Steal<Body<'_>> {
    let mut body = tcx.build_mir(def);

    pass_manager::dump_mir_for_phase_change(tcx, &body);

    pm::run_passes(
        tcx,
        &mut body,
        &[
            // MIR-level lints.
            &Lint(check_packed_ref::CheckPackedRef),
            &Lint(check_const_item_mutation::CheckConstItemMutation),
            &Lint(function_item_references::FunctionItemReferences),
            &Lint(check_undefined_transmutes::CheckUndefinedTransmutes),
            // What we need to do constant evaluation.
            &simplify::SimplifyCfg::Initial,
            &Lint(sanity_check::SanityCheck),
        ],
        None,
    );
    tcx.alloc_steal_mir(body)
}

/// Compute the main MIR body and the list of MIR bodies of the promoteds.
fn mir_promoted(
    tcx: TyCtxt<'_>,
    def: LocalDefId,
) -> (&Steal<Body<'_>>, &Steal<IndexVec<Promoted, Body<'_>>>) {
    // Ensure that we compute the `mir_const_qualif` for constants at
    // this point, before we steal the mir-const result.
    // Also this means promotion can rely on all const checks having been done.

    let const_qualifs = match tcx.def_kind(def) {
        DefKind::Fn | DefKind::AssocFn | DefKind::Closure
            if tcx.constness(def) == hir::Constness::Const
                || tcx.is_const_default_method(def.to_def_id()) =>
        {
            tcx.mir_const_qualif(def)
        }
        DefKind::AssocConst
        | DefKind::Const
        | DefKind::Static { .. }
        | DefKind::InlineConst
        | DefKind::AnonConst => tcx.mir_const_qualif(def),
        _ => ConstQualifs::default(),
    };

    // the `has_ffi_unwind_calls` query uses the raw mir, so make sure it is run.
    tcx.ensure_with_value().has_ffi_unwind_calls(def);

    // the `by_move_body` query uses the raw mir, so make sure it is run.
    if tcx.needs_coroutine_by_move_body_def_id(def.to_def_id()) {
        tcx.ensure_with_value().coroutine_by_move_body_def_id(def);
    }

    let mut body = tcx.mir_built(def).steal();
    if let Some(error_reported) = const_qualifs.tainted_by_errors {
        body.tainted_by_errors = Some(error_reported);
    }

    // Collect `required_consts` *before* promotion, so if there are any consts being promoted
    // we still add them to the list in the outer MIR body.
    RequiredConstsVisitor::compute_required_consts(&mut body);

    // What we need to run borrowck etc.
    let promote_pass = promote_consts::PromoteTemps::default();
    pm::run_passes(
        tcx,
        &mut body,
        &[&promote_pass, &simplify::SimplifyCfg::PromoteConsts, &coverage::InstrumentCoverage],
        Some(MirPhase::Analysis(AnalysisPhase::Initial)),
    );

    let promoted = promote_pass.promoted_fragments.into_inner();
    (tcx.alloc_steal_mir(body), tcx.alloc_steal_promoted(promoted))
}

/// Compute the MIR that is used during CTFE (and thus has no optimizations run on it)
fn mir_for_ctfe(tcx: TyCtxt<'_>, def_id: LocalDefId) -> &Body<'_> {
    tcx.arena.alloc(inner_mir_for_ctfe(tcx, def_id))
}

fn inner_mir_for_ctfe(tcx: TyCtxt<'_>, def: LocalDefId) -> Body<'_> {
    // FIXME: don't duplicate this between the optimized_mir/mir_for_ctfe queries
    if tcx.is_constructor(def.to_def_id()) {
        // There's no reason to run all of the MIR passes on constructors when
        // we can just output the MIR we want directly. This also saves const
        // qualification and borrow checking the trouble of special casing
        // constructors.
        return shim::build_adt_ctor(tcx, def.to_def_id());
    }

    let body = tcx.mir_drops_elaborated_and_const_checked(def);
    let body = match tcx.hir().body_const_context(def) {
        // consts and statics do not have `optimized_mir`, so we can steal the body instead of
        // cloning it.
        Some(hir::ConstContext::Const { .. } | hir::ConstContext::Static(_)) => body.steal(),
        Some(hir::ConstContext::ConstFn) => body.borrow().clone(),
        None => bug!("`mir_for_ctfe` called on non-const {def:?}"),
    };

    let mut body = remap_mir_for_const_eval_select(tcx, body, hir::Constness::Const);
    pm::run_passes(tcx, &mut body, &[&ctfe_limit::CtfeLimit], None);

    body
}

/// Obtain just the main MIR (no promoteds) and run some cleanups on it. This also runs
/// mir borrowck *before* doing so in order to ensure that borrowck can be run and doesn't
/// end up missing the source MIR due to stealing happening.
fn mir_drops_elaborated_and_const_checked(tcx: TyCtxt<'_>, def: LocalDefId) -> &Steal<Body<'_>> {
    if tcx.is_coroutine(def.to_def_id()) {
        tcx.ensure_with_value().mir_coroutine_witnesses(def);
    }

    // We only need to borrowck non-synthetic MIR.
    let tainted_by_errors =
        if !tcx.is_synthetic_mir(def) { tcx.mir_borrowck(def).tainted_by_errors } else { None };

    let is_fn_like = tcx.def_kind(def).is_fn_like();
    if is_fn_like {
        // Do not compute the mir call graph without said call graph actually being used.
        if pm::should_run_pass(tcx, &inline::Inline) {
            tcx.ensure_with_value().mir_inliner_callees(ty::InstanceKind::Item(def.to_def_id()));
        }
    }

    let (body, _) = tcx.mir_promoted(def);
    let mut body = body.steal();

    if let Some(error_reported) = tainted_by_errors {
        body.tainted_by_errors = Some(error_reported);
    }

    // Check if it's even possible to satisfy the 'where' clauses
    // for this item.
    //
    // This branch will never be taken for any normal function.
    // However, it's possible to `#!feature(trivial_bounds)]` to write
    // a function with impossible to satisfy clauses, e.g.:
    // `fn foo() where String: Copy {}`
    //
    // We don't usually need to worry about this kind of case,
    // since we would get a compilation error if the user tried
    // to call it. However, since we optimize even without any
    // calls to the function, we need to make sure that it even
    // makes sense to try to evaluate the body.
    //
    // If there are unsatisfiable where clauses, then all bets are
    // off, and we just give up.
    //
    // We manually filter the predicates, skipping anything that's not
    // "global". We are in a potentially generic context
    // (e.g. we are evaluating a function without instantiating generic
    // parameters, so this filtering serves two purposes:
    //
    // 1. We skip evaluating any predicates that we would
    // never be able prove are unsatisfiable (e.g. `<T as Foo>`
    // 2. We avoid trying to normalize predicates involving generic
    // parameters (e.g. `<T as Foo>::MyItem`). This can confuse
    // the normalization code (leading to cycle errors), since
    // it's usually never invoked in this way.
    let predicates = tcx
        .predicates_of(body.source.def_id())
        .predicates
        .iter()
        .filter_map(|(p, _)| if p.is_global() { Some(*p) } else { None });
    if traits::impossible_predicates(tcx, traits::elaborate(tcx, predicates).collect()) {
        trace!("found unsatisfiable predicates for {:?}", body.source);
        // Clear the body to only contain a single `unreachable` statement.
        let bbs = body.basic_blocks.as_mut();
        bbs.raw.truncate(1);
        bbs[START_BLOCK].statements.clear();
        bbs[START_BLOCK].terminator_mut().kind = TerminatorKind::Unreachable;
        body.var_debug_info.clear();
        body.local_decls.raw.truncate(body.arg_count + 1);
    }

    run_analysis_to_runtime_passes(tcx, &mut body);

    // Now that drop elaboration has been performed, we can check for
    // unconditional drop recursion.
    rustc_mir_build::lints::check_drop_recursion(tcx, &body);

    tcx.alloc_steal_mir(body)
}

// Made public so that `mir_drops_elaborated_and_const_checked` can be overridden
// by custom rustc drivers, running all the steps by themselves. See #114628.
pub fn run_analysis_to_runtime_passes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    assert!(body.phase == MirPhase::Analysis(AnalysisPhase::Initial));
    let did = body.source.def_id();

    debug!("analysis_mir_cleanup({:?})", did);
    run_analysis_cleanup_passes(tcx, body);
    assert!(body.phase == MirPhase::Analysis(AnalysisPhase::PostCleanup));

    // Do a little drop elaboration before const-checking if `const_precise_live_drops` is enabled.
    if check_consts::post_drop_elaboration::checking_enabled(&ConstCx::new(tcx, body)) {
        pm::run_passes(
            tcx,
            body,
            &[
                &remove_uninit_drops::RemoveUninitDrops,
                &simplify::SimplifyCfg::RemoveFalseEdges,
                &Lint(post_drop_elaboration::CheckLiveDrops),
            ],
            None,
        );
    }

    debug!("runtime_mir_lowering({:?})", did);
    run_runtime_lowering_passes(tcx, body);
    assert!(body.phase == MirPhase::Runtime(RuntimePhase::Initial));

    debug!("runtime_mir_cleanup({:?})", did);
    run_runtime_cleanup_passes(tcx, body);
    assert!(body.phase == MirPhase::Runtime(RuntimePhase::PostCleanup));
}

// FIXME(JakobDegen): Can we make these lists of passes consts?

/// After this series of passes, no lifetime analysis based on borrowing can be done.
fn run_analysis_cleanup_passes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let passes: &[&dyn MirPass<'tcx>] = &[
        &cleanup_post_borrowck::CleanupPostBorrowck,
        &remove_noop_landing_pads::RemoveNoopLandingPads,
        &simplify::SimplifyCfg::PostAnalysis,
        &deref_separator::Derefer,
    ];

    pm::run_passes(tcx, body, passes, Some(MirPhase::Analysis(AnalysisPhase::PostCleanup)));
}

/// Returns the sequence of passes that lowers analysis to runtime MIR.
fn run_runtime_lowering_passes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let passes: &[&dyn MirPass<'tcx>] = &[
        // These next passes must be executed together.
        &add_call_guards::CriticalCallEdges,
        // Must be done before drop elaboration because we need to drop opaque types, too.
        &reveal_all::RevealAll,
        // Calling this after reveal_all ensures that we don't deal with opaque types.
        &add_subtyping_projections::Subtyper,
        &elaborate_drops::ElaborateDrops,
        // This will remove extraneous landing pads which are no longer
        // necessary as well as forcing any call in a non-unwinding
        // function calling a possibly-unwinding function to abort the process.
        &abort_unwinding_calls::AbortUnwindingCalls,
        // AddMovesForPackedDrops needs to run after drop
        // elaboration.
        &add_moves_for_packed_drops::AddMovesForPackedDrops,
        // `AddRetag` needs to run after `ElaborateDrops` but before `ElaborateBoxDerefs`.
        // Otherwise it should run fairly late, but before optimizations begin.
        &add_retag::AddRetag,
        &elaborate_box_derefs::ElaborateBoxDerefs,
        &coroutine::StateTransform,
        &Lint(known_panics_lint::KnownPanicsLint),
    ];
    pm::run_passes_no_validate(tcx, body, passes, Some(MirPhase::Runtime(RuntimePhase::Initial)));
}

/// Returns the sequence of passes that do the initial cleanup of runtime MIR.
fn run_runtime_cleanup_passes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let passes: &[&dyn MirPass<'tcx>] = &[
        &lower_intrinsics::LowerIntrinsics,
        &remove_place_mention::RemovePlaceMention,
        &simplify::SimplifyCfg::PreOptimizations,
    ];

    pm::run_passes(tcx, body, passes, Some(MirPhase::Runtime(RuntimePhase::PostCleanup)));

    // Clear this by anticipation. Optimizations and runtime MIR have no reason to look
    // into this information, which is meant for borrowck diagnostics.
    for decl in &mut body.local_decls {
        decl.local_info = ClearCrossCrate::Clear;
    }
}

fn run_optimization_passes<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    fn o1<T>(x: T) -> WithMinOptLevel<T> {
        WithMinOptLevel(1, x)
    }

    // The main optimizations that we do on MIR.
    pm::run_passes(
        tcx,
        body,
        &[
            // Add some UB checks before any UB gets optimized away.
            &check_alignment::CheckAlignment,
            // Before inlining: trim down MIR with passes to reduce inlining work.

            // Has to be done before inlining, otherwise actual call will be almost always inlined.
            // Also simple, so can just do first.
            &lower_slice_len::LowerSliceLenCalls,
            // Perform instsimplify before inline to eliminate some trivial calls (like clone
            // shims).
            &instsimplify::InstSimplify::BeforeInline,
            // Perform inlining, which may add a lot of code.
            &inline::Inline,
            // Code from other crates may have storage markers, so this needs to happen after
            // inlining.
            &remove_storage_markers::RemoveStorageMarkers,
            // Inlining and instantiation may introduce ZST and useless drops.
            &remove_zsts::RemoveZsts,
            &remove_unneeded_drops::RemoveUnneededDrops,
            // Type instantiation may create uninhabited enums.
            // Also eliminates some unreachable branches based on variants of enums.
            &unreachable_enum_branching::UnreachableEnumBranching,
            &unreachable_prop::UnreachablePropagation,
            &o1(simplify::SimplifyCfg::AfterUnreachableEnumBranching),
            // Inlining may have introduced a lot of redundant code and a large move pattern.
            // Now, we need to shrink the generated MIR.
            &ref_prop::ReferencePropagation,
            &sroa::ScalarReplacementOfAggregates,
            &match_branches::MatchBranchSimplification,
            // inst combine is after MatchBranchSimplification to clean up Ne(_1, false)
            &multiple_return_terminators::MultipleReturnTerminators,
            // After simplifycfg, it allows us to discover new opportunities for peephole
            // optimizations.
            &instsimplify::InstSimplify::AfterSimplifyCfg,
            &simplify::SimplifyLocals::BeforeConstProp,
            &dead_store_elimination::DeadStoreElimination::Initial,
            &gvn::GVN,
            &simplify::SimplifyLocals::AfterGVN,
            &dataflow_const_prop::DataflowConstProp,
            &single_use_consts::SingleUseConsts,
            &o1(simplify_branches::SimplifyConstCondition::AfterConstProp),
            &jump_threading::JumpThreading,
            &early_otherwise_branch::EarlyOtherwiseBranch,
            &simplify_comparison_integral::SimplifyComparisonIntegral,
            &dest_prop::DestinationPropagation,
            &o1(simplify_branches::SimplifyConstCondition::Final),
            &o1(remove_noop_landing_pads::RemoveNoopLandingPads),
            &o1(simplify::SimplifyCfg::Final),
            &copy_prop::CopyProp,
            &dead_store_elimination::DeadStoreElimination::Final,
            &nrvo::RenameReturnPlace,
            &simplify::SimplifyLocals::Final,
            &multiple_return_terminators::MultipleReturnTerminators,
            &deduplicate_blocks::DeduplicateBlocks,
            &large_enums::EnumSizeOpt { discrepancy: 128 },
            // Some cleanup necessary at least for LLVM and potentially other codegen backends.
            &add_call_guards::CriticalCallEdges,
            // Cleanup for human readability, off by default.
            &prettify::ReorderBasicBlocks,
            &prettify::ReorderLocals,
            // Dump the end result for testing and debugging purposes.
            &dump_mir::Marker("PreCodegen"),
            &parmir::ParReEmit,
        ],
        Some(MirPhase::Runtime(RuntimePhase::Optimized)),
    );
}

/// Optimize the MIR and prepare it for codegen.
fn optimized_mir(tcx: TyCtxt<'_>, did: LocalDefId) -> &Body<'_> {
    tcx.arena.alloc(inner_optimized_mir(tcx, did))
}

fn inner_optimized_mir(tcx: TyCtxt<'_>, did: LocalDefId) -> Body<'_> {
    if tcx.is_constructor(did.to_def_id()) {
        // There's no reason to run all of the MIR passes on constructors when
        // we can just output the MIR we want directly. This also saves const
        // qualification and borrow checking the trouble of special casing
        // constructors.
        return shim::build_adt_ctor(tcx, did.to_def_id());
    }

    match tcx.hir().body_const_context(did) {
        // Run the `mir_for_ctfe` query, which depends on `mir_drops_elaborated_and_const_checked`
        // which we are going to steal below. Thus we need to run `mir_for_ctfe` first, so it
        // computes and caches its result.
        Some(hir::ConstContext::ConstFn) => tcx.ensure_with_value().mir_for_ctfe(did),
        None => {}
        Some(other) => panic!("do not use `optimized_mir` for constants: {other:?}"),
    }
    debug!("about to call mir_drops_elaborated...");
    let body = tcx.mir_drops_elaborated_and_const_checked(did).steal();
    let mut body = remap_mir_for_const_eval_select(tcx, body, hir::Constness::NotConst);

    if body.tainted_by_errors.is_some() {
        return body;
    }

    // Before doing anything, remember which items are being mentioned so that the set of items
    // visited does not depend on the optimization level.
    // We do not use `run_passes` for this as that might skip the pass if `injection_phase` is set.
    mentioned_items::MentionedItems.run_pass(tcx, &mut body);

    // If `mir_drops_elaborated_and_const_checked` found that the current body has unsatisfiable
    // predicates, it will shrink the MIR to a single `unreachable` terminator.
    // More generally, if MIR is a lone `unreachable`, there is nothing to optimize.
    if let TerminatorKind::Unreachable = body.basic_blocks[START_BLOCK].terminator().kind
        && body.basic_blocks[START_BLOCK].statements.is_empty()
    {
        return body;
    }

    run_optimization_passes(tcx, &mut body);

    body
}

/// Fetch all the promoteds of an item and prepare their MIR bodies to be ready for
/// constant evaluation once all generic parameters become known.
fn promoted_mir(tcx: TyCtxt<'_>, def: LocalDefId) -> &IndexVec<Promoted, Body<'_>> {
    if tcx.is_constructor(def.to_def_id()) {
        return tcx.arena.alloc(IndexVec::new());
    }

    if !tcx.is_synthetic_mir(def) {
        tcx.ensure_with_value().mir_borrowck(def);
    }
    let mut promoted = tcx.mir_promoted(def).1.steal();

    for body in &mut promoted {
        run_analysis_to_runtime_passes(tcx, body);
    }

    tcx.arena.alloc(promoted)
}
