//! Token types for the Xulo front end.
//!
//! This crate is types only: the lexical rules live in the `token` layer of
//! `xulo-parser/grammar.pest`, and tokenization is `xulo_parser::tokenize`.
//! What stays here is the [`token`] vocabulary the formatter, semantic
//! analyzer and editor share.

pub mod token;
