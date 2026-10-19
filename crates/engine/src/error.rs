use thiserror::Error;

/// Every failure a command can report. None of these abort the app.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    #[error("bad parameters: {0}")]
    BadParams(String),
    #[error("{0}")]
    NotFound(String),
    #[error("nothing is selected")]
    NothingSelected,
    #[error("cannot plan the job: {0}")]
    Plan(#[from] laserfoundry_toolpath::PlanError),
    #[error("cannot generate G-code: {0}")]
    Gcode(#[from] laserfoundry_gcode::GcodeError),
    #[error("file error: {0}")]
    File(#[from] laserfoundry_format::FormatError),
    #[error("could not import: {0}")]
    Import(String),
    #[error("device error: {0}")]
    Device(#[from] laserfoundry_device::DeviceError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error (your document was restored): {0}")]
    Internal(String),
}
