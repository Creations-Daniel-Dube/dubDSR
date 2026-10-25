#[derive(Debug, Clone)]
pub struct PipelineError {
    pub key: &'static str,
    pub args: Vec<String>,
}

impl PipelineError {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    pub fn with(key: &'static str, arg: impl Into<String>) -> Self {
        Self {
            key,
            args: vec![arg.into()],
        }
    }
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PipelineError {}
