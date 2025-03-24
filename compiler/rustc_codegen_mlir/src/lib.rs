#![allow(unused)]
#![allow(internal_features)]
#![allow(broken_intra_doc_links)]
#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_attr;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_errors;
extern crate rustc_fluent_macro;
extern crate rustc_fs_util;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_macros;
extern crate rustc_metadata;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

#[allow(unused_extern_crates)]
extern crate rustc_driver;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::{CodegenResults, CompiledModule, ModuleCodegen};
use rustc_data_structures::fx::FxIndexMap;
use rustc_errors::ErrorGuaranteed;
use rustc_metadata::EncodedMetadata;
use rustc_middle::dep_graph::{self, WorkProduct, WorkProductId};
use rustc_middle::infer::canonical::ir::ConstKind::Value;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_session::Session;
use rustc_session::config::OutputFilenames;

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

mod base;

#[derive(Clone)]
pub struct MLIRCodegenBackend(());

struct CodegenCarrier {}

impl CodegenBackend for MLIRCodegenBackend {
    fn locale_resource(&self) -> &'static str {
        crate::DEFAULT_LOCALE_RESOURCE
    }

    fn codegen_crate<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        metadata: EncodedMetadata,
        need_metadata_module: bool,
    ) -> Box<dyn std::any::Any> {
        println!("doing crate codegen");

        let (defids, codegen_units) = tcx.collect_and_partition_mono_items(());

        if tcx.dep_graph.is_fully_enabled() {
            for cgu in codegen_units {
                // TODO: do I need to use `ensure()`? since demand should be sufficient, no?
                //let c1 = tcx.codegen_unit(cgu.name());
                tcx.ensure().codegen_unit(cgu.name());
            }
        } else {
            println!("dep graph not fully enabled");
        }

        for (i, cgu) in codegen_units.iter().enumerate() {
            let s = cgu.name();
            println!("looking at cgu: {s}");
            let dep_node = cgu.codegen_dep_node(tcx);
            let module = tcx.dep_graph.with_task(
                dep_node,
                tcx,
                cgu.name(),
                base::module_codegen,
                Some(dep_graph::hash_result),
            );
        }

        todo!()
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn std::any::Any>,
        sess: &Session,
        outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        println!("doing join codegen");

        todo!()
    }

    fn link(
        &self,
        sess: &Session,
        codegen_results: CodegenResults,
        outputs: &OutputFilenames,
    ) -> Result<(), ErrorGuaranteed> {
        println!("doing link codegen");

        //use rustc_codegen_ssa::back::link::link_binary;

        //use crate::back::archive::LlvmArchiveBuilderBuilder;

        // Run the linker on any artifacts that resulted from the LLVM run.
        // This should produce either a finished executable or library.
        //link_binary(sess, &LlvmArchiveBuilderBuilder, &codegen_results, outputs)

        println!("unimplemented linking");

        Ok(())
    }

    fn supports_parallel(&self) -> bool {
        false
    }
}

#[no_mangle]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(MLIRCodegenBackend(()))
}
