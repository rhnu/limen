use std::{
    cell::RefCell,
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
};

use rquickjs::{
    Ctx, Error, Module,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};

use crate::{
    ScriptError, ScriptErrorKind, builtins,
    error::exception::{self, Location},
};

pub(crate) fn module_path(path: &Path) -> Result<PathBuf, ScriptError> {
    if !matches!(
        path.extension().and_then(|v| v.to_str()),
        Some("js" | "mjs")
    ) {
        return Err(ScriptError::new(ScriptErrorKind::Module));
    }
    path.canonicalize()
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Module, error))
}

/// Check metadata before opening so a named pipe cannot block the owning thread.
pub(crate) fn source(path: &Path, remaining: usize) -> Result<Vec<u8>, ScriptError> {
    let metadata = path
        .metadata()
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Module, error))?;
    if !metadata.is_file() || metadata.len() > u64::try_from(remaining).unwrap_or(u64::MAX) {
        return Err(ScriptError::new(ScriptErrorKind::Module));
    }
    let file = File::open(path)
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Module, error))?;
    if !file
        .metadata()
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Module, error))?
        .is_file()
    {
        return Err(ScriptError::new(ScriptErrorKind::Module));
    }
    let mut bytes = Vec::new();
    file.take(
        u64::try_from(remaining)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    )
    .read_to_end(&mut bytes)
    .map_err(|error| ScriptError::with_source(ScriptErrorKind::Module, error))?;
    if bytes.len() > remaining {
        return Err(ScriptError::new(ScriptErrorKind::Module));
    }
    Ok(bytes)
}

/// Loader callbacks must return `QuickJS` errors; retain their owned I/O causes separately.
#[derive(Clone, Default)]
pub(crate) struct ModuleDiagnostics(Rc<RefCell<ModuleState>>);

#[derive(Default)]
struct ModuleState {
    failure: Option<ScriptError>,
    names: Vec<String>,
}

impl ModuleDiagnostics {
    pub fn take(&self) -> Option<ScriptError> { self.0.borrow_mut().failure.take() }

    fn retain(&self, error: ScriptError) { self.0.borrow_mut().failure = Some(error); }

    pub fn register(&self, name: &str) {
        let mut state = self.0.borrow_mut();
        if !state.names.iter().any(|known| known == name) {
            state.names.push(name.to_owned());
        }
    }

    pub fn locate(&self, stack: &str) -> Option<Location> {
        exception::locate(stack, &self.0.borrow().names)
    }
}

pub(crate) struct ModuleResolver(pub ModuleDiagnostics);

impl Resolver for ModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let reject = || Error::new_resolving("script module", "module import");
        if attributes.is_none()
            && let Some(module) = builtins::find(name)
        {
            return Ok(module.name.to_owned());
        }
        if attributes.is_some()
            || !(name.starts_with("./") || name.starts_with("../"))
            || name.contains('\\')
            || name.contains(':')
            || name.contains(['?', '#'])
        {
            return Err(reject());
        }
        let parent = Path::new(base).parent().ok_or_else(reject)?;
        let path = module_path(&parent.join(name)).map_err(|error| {
            self.0.retain(error);
            reject()
        })?;
        path.to_str().map(str::to_owned).ok_or_else(reject)
    }
}

pub(crate) struct ModuleLoader {
    pub diagnostics: ModuleDiagnostics,
    pub remaining: usize,
    pub max_modules: usize,
    pub max_host_bytes: usize,
    pub loaded: HashSet<String>,
}

impl Loader for ModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let reject = || Error::new_loading("script module");
        if attributes.is_some() || self.loaded.len() >= self.max_modules {
            return Err(reject());
        }
        if let Some(module) = builtins::find(name) {
            if module.source_len() > self.remaining {
                return Err(reject());
            }
            self.remaining -= module.source_len();
            self.loaded.insert(name.to_owned());
            self.diagnostics.register(name);
            return module.declare(ctx, self.max_host_bytes);
        }
        let path = module_path(Path::new(name)).map_err(|error| {
            self.diagnostics.retain(error);
            reject()
        })?;
        if path != Path::new(name) {
            return Err(reject());
        }
        let bytes = source(&path, self.remaining).map_err(|error| {
            self.diagnostics.retain(error);
            reject()
        })?;
        self.remaining -= bytes.len();
        self.loaded.insert(name.to_owned());
        self.diagnostics.register(name);
        Module::declare(ctx.clone(), name, bytes)
    }
}
