use serde_json::Value;

use super::{ContentFrame, Kind, Target};

pub(super) fn frame(frame: &ContentFrame) -> (usize, usize) {
    let mut usage = (1_usize, std::mem::size_of::<ContentFrame>());
    add(&mut usage, value(&frame.source));
    for event in &frame.events {
        add(&mut usage, value(event));
    }
    for ((_, id), binding) in &frame.bindings {
        add(&mut usage, value(&binding.original));
        add(&mut usage, (1, 256_usize.saturating_add(id.capacity())));
        let bytes = match &binding.target {
            Target::Field(field) => field.capacity(),
            Target::Block { field, path, leaf } => field
                .capacity()
                .saturating_add(path.capacity().saturating_mul(std::mem::size_of::<usize>()))
                .saturating_add(leaf.capacity()),
            Target::Tool { kind, .. } => kind.capacity(),
            Target::Legacy => 0,
        };
        add(&mut usage, (0, bytes));
        if let Kind::Tool { kind, .. } = &binding.kind {
            add(&mut usage, (0, kind.capacity()));
        }
    }
    usage
}

fn value(input: &Value) -> (usize, usize) {
    let mut usage = (1, 256);
    match input {
        Value::String(text) => add(&mut usage, (0, text.capacity())),
        Value::Array(items) => {
            add(
                &mut usage,
                (
                    0,
                    items
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Value>()),
                ),
            );
            for item in items {
                add(&mut usage, value(item));
            }
        },
        Value::Object(fields) => {
            for (key, item) in fields {
                add(&mut usage, (1, 256_usize.saturating_add(key.capacity())));
                add(&mut usage, value(item));
            }
        },
        _ => {},
    }
    usage
}

fn add(usage: &mut (usize, usize), extra: (usize, usize)) {
    usage.0 = usage.0.saturating_add(extra.0);
    usage.1 = usage.1.saturating_add(extra.1);
}
