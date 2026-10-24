pub mod admission;
pub mod atmosphere;
mod atmosphere_fields;
pub mod atmosphere_transfer;
mod atmospheric_spectral_fields;
pub mod atmospheric_transport;
mod atmospheric_transport_receipt;
#[cfg(test)]
mod atmospheric_transport_tests;
pub mod authority;
pub mod cad;
pub mod cad_mesh;
pub mod cad_source;
pub mod cad_spectral;
pub mod cad_transport;
pub mod cad_transport_fields;
pub mod cad_transport_results;
pub mod cad_triangles;
pub mod cad_variant;
pub mod contact;
mod contact_fields;
pub mod contracts;
pub mod cooling_execution;
#[cfg(test)]
mod cooling_execution_tests;
mod cooling_fields;
pub mod cooling_results;
mod cooling_vtk;
pub mod devices;
pub mod estimates;
pub mod execution;
pub mod fem;
pub mod fem_imported;
pub mod fields;
pub mod filters;
pub mod frames;
pub mod freezing;
mod freezing_fields;
pub mod freezing_results;
mod json;
pub mod lifecycle;
pub mod materials;
mod measurements;
pub mod moisture_results;
pub mod presentation;
pub mod qualification;
pub mod radiation;
pub mod recipes;
pub mod resources;
pub mod results;
pub mod retained_cooling;
pub mod retention;
pub mod sandbox;
pub mod science;
pub mod snow;
pub mod snow_openings;
mod spectral_fields;
pub mod storage;
pub mod study;
pub mod thermal;
pub mod thermal_contact;
pub mod thermal_results;
pub mod thermal_transfer;
pub mod transfers;
pub mod wetting;
mod wetting_fields;
pub mod wetting_retention;
pub mod worker;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unqualified capability: {0}")]
    Unqualified(String),
    #[error("idempotency key already belongs to another immutable plan or host profile")]
    IdempotencyConflict,
    #[error("resource unavailable: {0}")]
    Resource(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: match self {
                Self::Invalid(_) => "invalid_input",
                Self::Unqualified(_) => "unqualified",
                Self::IdempotencyConflict => "idempotency_conflict",
                Self::Resource(_) => "resource_unavailable",
                Self::Io(_) => "io_error",
                Self::Json(_) => "protocol_error",
                Self::Sql(_) => "state_error",
            },
            message: self.to_string(),
        }
    }
}
