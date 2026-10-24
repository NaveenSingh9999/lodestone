//! Termux repository provisioning of Java runtimes (spec §4.2):
//! InRelease signature verification -> Packages fetch/verify -> dependency
//! closure -> package fetch/verify -> prefix-stripped extraction.

pub mod deb;
pub mod keys;
pub mod provision;
pub mod repo;
pub mod resolve;
pub mod verify;
pub mod version;
