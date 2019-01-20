/// tar archive format definitions and serializers.
pub mod tar;

/// Multithreaded path traversal (the thing which makes rapidtar rapid).
pub mod traverse;

/// Implementation of tar's fixed-size record buffered writer.
pub mod blocking;

/// Abstraction layer for platform-specific magnetic tape behaviors.
pub mod tape;

/// Abstraction layer for platform-specific behaviors rapidtar needs.
pub mod fs;

/// Platform-agnostic path normalization compatible with tar behavior.
pub mod normalize;

/// Facilities for tracking data within a write buffer for error recovery.
pub mod spanning;

/// Basic result types for functions which can partially succeed.
pub mod result;

/// Utilities for efficient I/O copies.
pub mod stream;
