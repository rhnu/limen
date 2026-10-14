use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use rquickjs::{
    Ctx, Error, Module,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};

use crate::ScriptError;

pub(crate) fn module_path(path: &Path) -> Result<PathBuf, ScriptError> {
    if !matches!(
        path.extension().and_then(|v| v.to_str()),
        Some("js" | "mjs")
    ) {
        return Err(ScriptError::Module);
    }
    path.canonicalize().map_err(|_error| ScriptError::Module)
}

/// Check metadata before opening so a named pipe cannot block the owning thread.
pub(crate) fn source(path: &Path, remaining: usize) -> Result<Vec<u8>, ScriptError> {
    let metadata = path.metadata().map_err(|_error| ScriptError::Module)?;
    if !metadata.is_file() || metadata.len() > u64::try_from(remaining).unwrap_or(u64::MAX) {
        return Err(ScriptError::Module);
    }
    let file = File::open(path).map_err(|_error| ScriptError::Module)?;
    if !file
        .metadata()
        .map_err(|_error| ScriptError::Module)?
        .is_file()
    {
        return Err(ScriptError::Module);
    }
    let mut bytes = Vec::new();
    file.take(
        u64::try_from(remaining)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    )
    .read_to_end(&mut bytes)
    .map_err(|_error| ScriptError::Module)?;
    if bytes.len() > remaining {
        return Err(ScriptError::Module);
    }
    Ok(bytes)
}

pub(crate) struct LocalResolver;

impl Resolver for LocalResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let reject = || Error::new_resolving("local module", "local import");
        if attributes.is_some()
            || !(name.starts_with("./") || name.starts_with("../"))
            || name.contains('\\')
            || name.contains(':')
            || name.contains(['?', '#'])
        {
            return Err(reject());
        }
        let parent = Path::new(base).parent().ok_or_else(reject)?;
        let path = module_path(&parent.join(name)).map_err(|_error| reject())?;
        path.to_str().map(str::to_owned).ok_or_else(reject)
    }
}

pub(crate) struct LocalLoader {
    pub remaining: usize,
    pub max_modules: usize,
    pub loaded: HashSet<PathBuf>,
}

impl Loader for LocalLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let reject = || Error::new_loading("local module");
        if attributes.is_some() || self.loaded.len() >= self.max_modules {
            return Err(reject());
        }
        let path = module_path(Path::new(name)).map_err(|_error| reject())?;
        if path != Path::new(name) {
            return Err(reject());
        }
        let bytes = source(&path, self.remaining).map_err(|_error| reject())?;
        self.remaining -= bytes.len();
        self.loaded.insert(path);
        Module::declare(ctx.clone(), name, bytes)
    }
}
