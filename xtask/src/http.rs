//! Local HTTP experiments with synthetic upstreams and parent-owned temporary files.

mod fixture;
mod lifecycle;
mod observations;
mod routing;
mod transport;
mod upstream;

use std::path::Path;

use anyhow::{Context, Result, bail};
use fixture::Laboratory;

pub const CASES: &[&str] = &[
    "auth-routing",
    "pipeline",
    "failures",
    "deadline",
    "disconnect",
    "shutdown",
    "retirement",
    "resources",
    "observations",
    "transport-contracts",
];

/// Run one externally supervised experiment, checking every fixture's logs on return.
pub async fn run(case: &str, executable: &Path, directory: &Path) -> Result<()> {
    let mut laboratory = Laboratory::new(executable, directory);
    let result = match case {
        "auth-routing" => routing::auth_routing(&mut laboratory).await,
        "pipeline" => routing::pipeline(&mut laboratory).await,
        "failures" => routing::failures(&mut laboratory).await,
        "deadline" => lifecycle::deadline(&mut laboratory).await,
        "disconnect" => lifecycle::disconnect(&mut laboratory).await,
        "shutdown" => lifecycle::shutdown(&mut laboratory).await,
        "retirement" => lifecycle::retirement(&mut laboratory).await,
        "resources" => lifecycle::resources(&mut laboratory).await,
        "observations" => observations::run(&mut laboratory).await,
        "transport-contracts" => transport::run(&mut laboratory).await,
        _ => bail!("unknown HTTP experiment: {case}"),
    };
    let safety = laboratory.check_logs();
    match (result, safety) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(primary), Err(cleanup)) => {
            Err(primary.context(format!("log check also failed: {cleanup:#}")))
        },
    }
    .with_context(|| format!("HTTP experiment {case}"))
}
