//! Native exports for the runtime builtin module; no host globals are published.

mod env;

use rquickjs::{
    Ctx, Function, Module,
    module::{Declarations, Declared, Exports, ModuleDef},
};

struct RuntimeModule;

impl ModuleDef for RuntimeModule {
    fn declare(declarations: &Declarations<'_>) -> rquickjs::Result<()> {
        declarations.declare("env")?;
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let env: Function = exports.module().meta()?.get("env")?;
        exports.export("env", env)?;
        Ok(())
    }
}

pub(crate) fn declare<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    max_bytes: usize,
) -> rquickjs::Result<Module<'js, Declared>> {
    let module = Module::declare_def::<RuntimeModule, _>(ctx.clone(), name)?;
    // Native evaluation has no loader state; private metadata retains this VM's bound function.
    module.meta()?.set("env", env::function(ctx, max_bytes)?)?;
    Ok(module)
}
