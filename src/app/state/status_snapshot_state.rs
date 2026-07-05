#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DcVoltageSnapshot {
    pub dc1: Option<f64>,
    pub dc2: Option<f64>,
}

impl DcVoltageSnapshot {
    pub fn has_warning(self) -> bool {
        [self.dc1, self.dc2]
            .into_iter()
            .any(|voltage| voltage.is_none_or(|voltage| !voltage.is_finite() || voltage < 10.0))
    }
}
