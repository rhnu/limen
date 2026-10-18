//! Fixed ESM registry. Adding a host module requires an explicit entry here.

use rquickjs::{Ctx, Module, module::Declared};

pub(crate) struct BuiltinModule {
    pub name: &'static str,
    body: Body,
}

enum Body {
    JavaScript(&'static [u8]),
    Runtime,
}

const MODULES: &[BuiltinModule] = &[
    BuiltinModule {
        name: "limen:gateway",
        body: Body::JavaScript(include_bytes!("../dist/gateway.js")),
    },
    BuiltinModule {
        name: "limen:providers",
        body: Body::JavaScript(include_bytes!("../dist/providers.js")),
    },
    BuiltinModule {
        name: "limen:policies",
        body: Body::JavaScript(include_bytes!("../dist/policies.js")),
    },
    BuiltinModule {
        name: "limen:content",
        body: Body::JavaScript(include_bytes!("../dist/content.js")),
    },
    BuiltinModule {
        name: "limen:runtime",
        body: Body::Runtime,
    },
];

pub(crate) fn find(name: &str) -> Option<&'static BuiltinModule> {
    MODULES.iter().find(|module| module.name == name)
}

impl BuiltinModule {
    pub fn source_len(&self) -> usize {
        match self.body {
            Body::JavaScript(source) => source.len(),
            Body::Runtime => 0,
        }
    }

    pub fn declare<'js>(
        &self,
        ctx: &Ctx<'js>,
        max_host_bytes: usize,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        match self.body {
            Body::JavaScript(source) => Module::declare(ctx.clone(), self.name, source),
            Body::Runtime => crate::host::declare(ctx, self.name, max_host_bytes),
        }
    }
}
