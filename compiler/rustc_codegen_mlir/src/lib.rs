#![allow(unused)]
#![allow(internal_features)]

use rustc_codegen_ssa::traits::*;
use rustc_codegen_ssa::{CodegenResults, CompiledModule, ModuleCodegen};
use rustc_data_structures::fx::FxIndexMap;
use rustc_errors::ErrorGuaranteed;
use rustc_metadata::EncodedMetadata;
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::infer::canonical::ir::ConstKind::Value;
use rustc_middle::ty::TyCtxt;
use rustc_session::Session;
use rustc_session::config::OutputFilenames;

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

#[derive(Clone)]
pub struct MLIRCodegenBackend(());

impl ExtraBackendMethods for MLIRCodegenBackend {
    fn codegen_allocator<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        module_name: &str,
        kind: rustc_ast::expand::allocator::AllocatorKind,
        alloc_error_handler_kind: rustc_ast::expand::allocator::AllocatorKind,
    ) -> Self::Module {
        todo!()
    }

    fn compile_codegen_unit(
        &self,
        tcx: TyCtxt<'_>,
        cgu_name: rustc_span::Symbol,
    ) -> (ModuleCodegen<Self::Module>, u64) {
        todo!()
    }

    fn target_machine_factory(
        &self,
        sess: &Session,
        opt_level: rustc_session::config::OptLevel,
        target_features: &[String],
    ) -> rustc_codegen_ssa::back::write::TargetMachineFactoryFn<Self> {
        todo!()
    }
}

impl WriteBackendMethods for MLIRCodegenBackend {
    type Module = ();

    type TargetMachine = ();

    type TargetMachineError = ();

    type ModuleBuffer;

    type ThinData = ();

    type ThinBuffer;

    fn run_link(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        dcx: rustc_errors::DiagCtxtHandle<'_>,
        modules: Vec<ModuleCodegen<Self::Module>>,
    ) -> Result<ModuleCodegen<Self::Module>, rustc_errors::FatalError> {
        todo!()
    }

    fn run_fat_lto(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        modules: Vec<rustc_codegen_ssa::back::write::FatLtoInput<Self>>,
        cached_modules: Vec<(
            rustc_codegen_ssa::back::lto::SerializedModule<Self::ModuleBuffer>,
            WorkProduct,
        )>,
    ) -> Result<rustc_codegen_ssa::back::lto::LtoModuleCodegen<Self>, rustc_errors::FatalError>
    {
        todo!()
    }

    fn run_thin_lto(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        modules: Vec<(String, Self::ThinBuffer)>,
        cached_modules: Vec<(
            rustc_codegen_ssa::back::lto::SerializedModule<Self::ModuleBuffer>,
            WorkProduct,
        )>,
    ) -> Result<
        (Vec<rustc_codegen_ssa::back::lto::LtoModuleCodegen<Self>>, Vec<WorkProduct>),
        rustc_errors::FatalError,
    > {
        todo!()
    }

    fn print_pass_timings(&self) {
        todo!()
    }

    fn print_statistics(&self) {
        todo!()
    }

    unsafe fn optimize(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        dcx: rustc_errors::DiagCtxtHandle<'_>,
        module: &ModuleCodegen<Self::Module>,
        config: &rustc_codegen_ssa::back::write::ModuleConfig,
    ) -> Result<(), rustc_errors::FatalError> {
        todo!()
    }

    fn optimize_fat(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        llmod: &mut ModuleCodegen<Self::Module>,
    ) -> Result<(), rustc_errors::FatalError> {
        todo!()
    }

    unsafe fn optimize_thin(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        thin: rustc_codegen_ssa::back::lto::ThinModule<Self>,
    ) -> Result<ModuleCodegen<Self::Module>, rustc_errors::FatalError> {
        todo!()
    }

    unsafe fn codegen(
        cgcx: &rustc_codegen_ssa::back::write::CodegenContext<Self>,
        dcx: rustc_errors::DiagCtxtHandle<'_>,
        module: ModuleCodegen<Self::Module>,
        config: &rustc_codegen_ssa::back::write::ModuleConfig,
    ) -> Result<CompiledModule, rustc_errors::FatalError> {
        todo!()
    }

    fn prepare_thin(
        module: ModuleCodegen<Self::Module>,
        want_summary: bool,
    ) -> (String, Self::ThinBuffer) {
        todo!()
    }

    fn serialize_module(module: ModuleCodegen<Self::Module>) -> (String, Self::ModuleBuffer) {
        todo!()
    }
}

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

        Box::new(rustc_codegen_ssa::base::codegen_crate(
            MLIRCodegenBackend(()),
            tcx,
            rustc_codegen_llvm::llvm_util::target_cpu(tcx.sess).to_string(),
            metadata,
            need_metadata_module,
        ))
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn std::any::Any>,
        sess: &Session,
        outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        println!("doing join codegen");

        let (codegen_results, work_products) = ongoing_codegen
            .downcast::<rustc_codegen_ssa::back::write::OngoingCodegen<MLIRCodegenBackend>>()
            .expect("Expected MLIRCodegenBackend's OngoingCodegen, found Box<Any>")
            .join(sess);

        (codegen_results, work_products)
    }

    fn link(
        &self,
        sess: &Session,
        codegen_results: CodegenResults,
        outputs: &OutputFilenames,
    ) -> Result<(), ErrorGuaranteed> {
        println!("doing link codegen");

        use rustc_codegen_ssa::back::link::link_binary;

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
