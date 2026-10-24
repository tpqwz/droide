//! Droide compiler. Forms become DEX, a binary manifest, and a signed APK.
//! The studio links this crate. Python remains only for the browser IDE and the Qt fallback.

pub mod apk;
pub mod ast;
pub mod axml;
pub mod build;
pub mod check;
pub mod codegen;
pub mod dex;
pub mod error;
pub mod lex;
pub mod native;
pub mod ndk;
pub mod parse;
pub mod project;
pub mod index;
pub mod search;
pub mod vm;
pub mod widgets;
