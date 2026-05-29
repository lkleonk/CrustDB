use crustdb_core::{CrustDbError, Engine as CoreEngine, ModelSchema};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::Mutex;

#[pyclass]
struct Engine {
    inner: Mutex<CoreEngine>,
}

#[pymethods]
impl Engine {
    #[new]
    fn new(path: String) -> PyResult<Self> {
        let engine = CoreEngine::open(path).map_err(to_py_err)?;
        Ok(Self {
            inner: Mutex::new(engine),
        })
    }

    fn register_schema(&self, schema_json: String) -> PyResult<()> {
        let schema: ModelSchema = serde_json::from_str(&schema_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("CrustDB engine lock was poisoned"))?;
        engine.register_schema(schema).map_err(to_py_err)
    }

    fn insert(&self, model_name: String, values_json: String) -> PyResult<String> {
        let values = serde_json::from_str(&values_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("CrustDB engine lock was poisoned"))?;
        let row = engine.insert(&model_name, values).map_err(to_py_err)?;
        serde_json::to_string(&row).map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn find(&self, model_name: String, filters_json: String) -> PyResult<Option<String>> {
        let filters = serde_json::from_str(&filters_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let engine = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("CrustDB engine lock was poisoned"))?;
        let row = engine.find(&model_name, &filters).map_err(to_py_err)?;
        row.map(|record| {
            serde_json::to_string(&record).map_err(|error| PyValueError::new_err(error.to_string()))
        })
        .transpose()
    }

    fn delete(&self, model_name: String, filters_json: String) -> PyResult<bool> {
        let filters = serde_json::from_str(&filters_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("CrustDB engine lock was poisoned"))?;
        engine.delete(&model_name, &filters).map_err(to_py_err)
    }

    fn update(
        &self,
        model_name: String,
        filters_json: String,
        values_json: String,
    ) -> PyResult<Option<String>> {
        let filters = serde_json::from_str(&filters_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let values = serde_json::from_str(&values_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("CrustDB engine lock was poisoned"))?;
        let row = engine
            .update(&model_name, &filters, values)
            .map_err(to_py_err)?;
        row.map(|record| {
            serde_json::to_string(&record).map_err(|error| PyValueError::new_err(error.to_string()))
        })
        .transpose()
    }
}

fn to_py_err(error: CrustDbError) -> PyErr {
    match error {
        CrustDbError::Validation(message) => PyValueError::new_err(message),
        CrustDbError::UniqueConstraint(message) => PyValueError::new_err(message),
        CrustDbError::IncompatibleSchema(message) => PyValueError::new_err(message),
        CrustDbError::UnknownModel(message) => PyValueError::new_err(message),
        CrustDbError::StorageFormat(message) => PyRuntimeError::new_err(message),
        CrustDbError::Storage(error) => PyRuntimeError::new_err(error.to_string()),
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Engine>()?;
    Ok(())
}
