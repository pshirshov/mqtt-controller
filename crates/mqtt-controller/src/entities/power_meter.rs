use crate::tass::TassActual;

#[derive(Debug, Clone, Default)]
pub struct PowerMeterEntity {
    pub power: TassActual<f64>,
    pub energy: TassActual<f64>,
}
