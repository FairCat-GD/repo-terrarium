#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub mod binding;
pub mod ci;
pub mod discovery;
pub mod exec;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod git;
pub mod local_tests;
pub mod metrics;
pub mod report;
