use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Symbol;

pub fn module_codegen(tcx: TyCtxt<'_>, cgu_name: Symbol) -> () {
    let cgu = tcx.codegen_unit(cgu_name);

    for (mono_item, data) in cgu.items() {
        match mono_item {
            rustc_middle::mir::mono::MonoItem::Fn(instance) => {
                println!("got fn!");
            },
            rustc_middle::mir::mono::MonoItem::Static(def_id) => {
                println!("got static item");
            },
            rustc_middle::mir::mono::MonoItem::GlobalAsm(item_id) => {
                println!("got global asm");
            },
        }
    }
}
