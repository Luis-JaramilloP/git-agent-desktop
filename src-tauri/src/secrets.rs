//! Almacen de la clave del proveedor de IA.
//!
//! Principio: **nunca una clave en texto plano**. La clave se cifra con DPAPI
//! (`CryptProtectData`), de modo que solo la cuenta de Windows que la guardo
//! puede descifrarla. El archivo `provider_key.dat` contiene unicamente el blob
//! cifrado en hexadecimal.
//!
//! Alternativa sin tocar disco: variable de entorno `GAD_API_KEY`, que tiene
//! prioridad sobre el valor almacenado.

use std::path::PathBuf;

use serde::Serialize;

use crate::config::AppConfig;

const ENV_KEY: &str = "GAD_API_KEY";
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    /// Hay una clave guardada (cifrada) en disco.
    pub stored: bool,
    /// La clave proviene de `GAD_API_KEY`.
    pub from_env: bool,
    /// El almacen cifrado esta disponible en esta plataforma.
    pub available: bool,
    pub path: String,
}

pub fn secret_path() -> PathBuf {
    AppConfig::config_dir().join("provider_key.dat")
}

pub fn env_key() -> Option<String> {
    std::env::var(ENV_KEY)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn has_stored() -> bool {
    secret_path().exists()
}

pub fn status() -> SecretStatus {
    SecretStatus {
        stored: has_stored(),
        from_env: env_key().is_some(),
        available: cfg!(windows),
        path: secret_path().to_string_lossy().to_string(),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        out.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if cleaned.is_empty() || cleaned.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for pair in cleaned.chunks(2) {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        out.push(((high << 4) | low) as u8);
    }
    Some(out)
}

/// Guarda la clave cifrada. Con una cadena vacia equivale a borrarla.
pub fn save(key: &str) -> Result<SecretStatus, String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return clear();
    }

    #[cfg(windows)]
    {
        let blob = dpapi::protect(trimmed.as_bytes())?;
        let dir = AppConfig::config_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("No se pudo crear {:?}: {}", dir, e))?;
        std::fs::write(secret_path(), to_hex(&blob))
            .map_err(|e| format!("No se pudo guardar la clave: {}", e))?;
        Ok(status())
    }

    #[cfg(not(windows))]
    {
        Err(
            "El almacen cifrado requiere Windows (DPAPI). Define la variable GAD_API_KEY."
                .to_string(),
        )
    }
}

/// Lee la clave (entorno primero, luego el blob cifrado).
pub fn load() -> Result<Option<String>, String> {
    if let Some(env) = env_key() {
        return Ok(Some(env));
    }

    if !has_stored() {
        return Ok(None);
    }

    let hex = std::fs::read_to_string(secret_path())
        .map_err(|e| format!("No se pudo leer la clave guardada: {}", e))?;
    let blob = from_hex(hex.trim()).ok_or_else(|| {
        "El archivo de la clave esta corrupto; vuelve a guardarla en Ajustes.".to_string()
    })?;

    #[cfg(windows)]
    {
        let plain = dpapi::unprotect(&blob)?;
        let key = String::from_utf8(plain)
            .map_err(|_| "La clave guardada no es texto UTF-8 valido.".to_string())?;
        Ok(Some(key))
    }

    #[cfg(not(windows))]
    {
        let _ = blob;
        Ok(None)
    }
}

pub fn clear() -> Result<SecretStatus, String> {
    let path = secret_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("No se pudo borrar la clave: {}", e))?;
    }
    Ok(status())
}

// ---------------------------------------------------------------------------
// DPAPI (solo Windows)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    /// Sin interfaz de usuario (nunca muestra un dialogo desde el backend).
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    fn copy_and_free(blob: DataBlob) -> Vec<u8> {
        let data = if blob.pb_data.is_null() || blob.cb_data == 0 {
            Vec::new()
        } else {
            // SAFETY: DPAPI devuelve un buffer valido de `cb_data` bytes.
            unsafe { std::slice::from_raw_parts(blob.pb_data, blob.cb_data as usize).to_vec() }
        };
        if !blob.pb_data.is_null() {
            // SAFETY: el buffer lo reservo DPAPI con LocalAlloc.
            unsafe {
                LocalFree(blob.pb_data as *mut c_void);
            }
        }
        data
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = DataBlob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };

        // SAFETY: los punteros apuntan a estructuras vivas durante la llamada.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };

        if ok == 0 {
            return Err(format!(
                "DPAPI no pudo cifrar la clave: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(copy_and_free(output))
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        let input = DataBlob {
            cb_data: cipher.len() as u32,
            pb_data: cipher.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };

        // SAFETY: idem `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };

        if ok == 0 {
            return Err(format!(
                "DPAPI no pudo descifrar la clave (otra cuenta de Windows o archivo alterado): {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(copy_and_free(output))
    }
}
