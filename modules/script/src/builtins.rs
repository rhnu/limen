//! Fixed ESM registry. Adding a host module requires an explicit entry here.

pub(crate) struct BuiltinModule {
    pub name: &'static str,
    body: &'static [u8],
}

const MODULES: &[BuiltinModule] = &[
    BuiltinModule {
        name: "limen:gateway",
        body: include_bytes!("../dist/gateway.js"),
    },
    BuiltinModule {
        name: "limen:policies",
        body: include_bytes!("../dist/policies.js"),
    },
    BuiltinModule {
        name: "limen:content",
        body: include_bytes!("../dist/content.js"),
    },
];

pub(crate) fn find(name: &str) -> Option<&'static BuiltinModule> {
    MODULES.iter().find(|module| module.name == name)
}

impl BuiltinModule {
    pub fn source_len(&self) -> usize { self.body.len() }

    pub fn source(&self) -> &'static [u8] { self.body }
}
