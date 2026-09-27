//! sizetree — WizTree-style disk usage analyzer.
//!
//! Library half: disk scanning (direct NTFS `$MFT` reads on Windows,
//! directory walk elsewhere), squarified treemap layout, bilingual strings,
//! and the small pure helpers the UI is built from.

pub mod i18n;
pub mod mft;
pub mod treemap;
pub mod util;
