//! Fixed ESM registry. Adding a host module requires an explicit entry here.

pub(crate) struct BuiltinModule {
    pub name: &'static str,
    body: &'static [u8],
}

const MODULES: &[BuiltinModule] = &[
    BuiltinModule {
        name: "limen:policies",
        body: include_bytes!("builtins/policies.js"),
    },
    BuiltinModule {
        name: "limen:content",
        body: include_bytes!("builtins/content.js"),
    },
];

pub(crate) fn find(name: &str) -> Option<&'static BuiltinModule> {
    MODULES.iter().find(|module| module.name == name)
}

// One authored constructor is evaluated privately within each builtin module.
const PRELUDE: &[u8] = include_bytes!("builtins/descriptor.js");

impl BuiltinModule {
    pub fn source_len(&self) -> usize { PRELUDE.len() + self.body.len() }

    pub fn source(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.source_len());
        bytes.extend_from_slice(PRELUDE);
        bytes.extend_from_slice(self.body);
        bytes
    }
}
