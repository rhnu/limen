//! Central registration and publication of script-visible host capabilities.

mod env;

use rquickjs::{Ctx, Function, Object};

/// Register capabilities before freezing and publishing the single global namespace.
/// Called only during pristine bootstrap, before user modules can change intrinsics.
pub(crate) fn install(ctx: &Ctx<'_>, max_bytes: usize) -> rquickjs::Result<()> {
    let namespace = Object::new(ctx.clone())?;

    namespace.prop("env", env::function(ctx, max_bytes)?)?;

    let freeze: Function = ctx.eval("Object.freeze")?;
    freeze.call::<_, Object>((namespace.clone(),))?;
    ctx.globals().prop("limen", namespace)
}
