use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Symbol;

pub fn module_codegen(tcx: TyCtxt<'_>, cgu_name: Symbol) -> () {
    let cgu = tcx.codegen_unit(cgu_name);

    for (mono_item, data) in cgu.items() {
        match mono_item {
            rustc_middle::mir::mono::MonoItem::Fn(instance) => {
                let mir = tcx.instance_mir(instance.def);

                println!("Got mir: {mir:?}");

                //instance.def_id().
                println!("fn instance is: {:?}", instance.def_id());
                match instance.def {
                    ty::InstanceKind::Item(def_id) => {
                        println!("item");
                    }
                    ty::InstanceKind::Intrinsic(def_id) => {
                        println!("intrinsic");
                    }
                    ty::InstanceKind::VTableShim(def_id) => {
                        println!("vtable shim");
                    }
                    ty::InstanceKind::ReifyShim(def_id, reify_reason) => {
                        println!("reify shim");
                    }
                    ty::InstanceKind::FnPtrShim(def_id, ty) => {
                        println!("fnptr shim");
                    }
                    ty::InstanceKind::Virtual(def_id, _) => todo!(),
                    ty::InstanceKind::ClosureOnceShim { call_once, track_caller } => {
                        println!("closure once shim");
                    }
                    ty::InstanceKind::ConstructCoroutineInClosureShim {
                        coroutine_closure_def_id,
                        receiver_by_ref,
                    } => {
                        println!("construct coroutine in closure shim");
                    }
                    ty::InstanceKind::ThreadLocalShim(def_id) => {
                        println!("thread local shim");
                    }
                    ty::InstanceKind::DropGlue(def_id, ty) => {
                        println!("drop glue");
                    }
                    ty::InstanceKind::CloneShim(def_id, ty) => {
                        println!("clone shim");
                    }
                    ty::InstanceKind::FnPtrAddrShim(def_id, ty) => {
                        println!("fn ptr addr shim");
                    }
                    ty::InstanceKind::AsyncDropGlueCtorShim(def_id, ty) => todo!(),
                }
                println!("got fn!");
            }
            rustc_middle::mir::mono::MonoItem::Static(def_id) => {
                println!("got static item");
            }
            rustc_middle::mir::mono::MonoItem::GlobalAsm(item_id) => {
                println!("got global asm");
            }
        }
    }
}
