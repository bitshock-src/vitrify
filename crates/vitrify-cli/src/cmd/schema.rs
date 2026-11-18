//! `vitrify schema show [--tdd]` — print the embedded normative schema
//! (disk override under `~/.vitrify/schemas/` wins).

use vitrify_core::CoreError;

use crate::home;
use crate::out::emit;

pub fn show(tdd: bool) -> Result<u8, CoreError> {
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    let rel = if tdd {
        "schemas/tdd-schema.md"
    } else {
        "schemas/sdd-schema.md"
    };
    let (body, _) = home::resolve(&home_dir, super::brief::VERSION, rel)?;
    emit(body.trim_end_matches('\n'));
    Ok(0)
}
