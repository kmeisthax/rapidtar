//! Code dealing with global headers, which we call labels.

/// Represents globally-applcable information for an entire tar archive file,
/// such as it's volume label.
#[derive(Clone)]
pub struct TarLabel {
    pub label: Option<String>,
    pub nabla: u64,
    pub volume_identifier: usize,
}

