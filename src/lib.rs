//! Anacleto — Agent orchestration engine in Rust.
//!
//! This library crate provides all modules for the Anacleto engine.
//! The binary entry point is in `main.rs`.

pub mod agent;
pub mod config;
pub mod db;
pub mod engine;
pub mod error;
pub mod filesystem;
pub mod llm;
pub mod lsp;
pub mod mcp;
pub mod permissions;
pub mod plugin;
pub mod shell;
pub mod skill;
pub mod tools;
pub mod tui;
