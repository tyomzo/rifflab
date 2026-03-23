/// Named bus identifiers for the audio graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusId {
    /// Hardware input.
    Input,
    /// A stem track (by index).
    Stem(usize),
    /// Post-effects return from the input.
    FxReturn,
    /// Final mix output.
    Master,
}
