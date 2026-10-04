#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod git;
