//! Lectura y escritura de patrones en JSON.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Pattern;

/// Identificador del formato, para reconocer los ficheros.
pub const FORMAT_ID: &str = "clothing-maker/pattern";
/// Versión actual del formato. Súbela cuando haya cambios incompatibles.
pub const FORMAT_VERSION: u32 = 1;

/// Envoltorio del fichero `.pattern.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PatternFile {
    pub format: String,
    pub version: u32,
    pub pattern: Pattern,
}

#[derive(Debug, thiserror::Error)]
pub enum PatternIoError {
    #[error("no se pudo leer o escribir el fichero: {0}")]
    Io(#[from] std::io::Error),
    #[error("el fichero no es un patrón válido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("el fichero no es un patrón de clothing-maker (formato «{0}»)")]
    WrongFormat(String),
    #[error("el fichero usa la versión {0} del formato y esta aplicación solo entiende hasta la {FORMAT_VERSION}")]
    UnsupportedVersion(u32),
}

impl Pattern {
    pub fn to_json(&self) -> Result<String, PatternIoError> {
        let file = PatternFile { format: FORMAT_ID.into(), version: FORMAT_VERSION, pattern: self.clone() };
        Ok(serde_json::to_string_pretty(&file)?)
    }

    pub fn from_json(json: &str) -> Result<Self, PatternIoError> {
        let file: PatternFile = serde_json::from_str(json)?;
        if file.format != FORMAT_ID {
            return Err(PatternIoError::WrongFormat(file.format));
        }
        if file.version > FORMAT_VERSION {
            return Err(PatternIoError::UnsupportedVersion(file.version));
        }
        let mut pattern = file.pattern;
        // Asegura que `next_id` no choca con ids existentes.
        let id = pattern.alloc_id();
        pattern.next_id = id;
        Ok(pattern)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatternIoError> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatternIoError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point, SeamSide};

    fn sample() -> Pattern {
        let mut p = Pattern::new("camiseta");
        let a = p.add_rect("delantero", Point::ZERO, 500.0, 700.0);
        let b = p.add_rect("espalda", Point::new(600.0, 0.0), 500.0, 700.0);
        p.piece_mut(a).unwrap().make_curve(0);
        p.piece_mut(a).unwrap().set_control_point(0, 0, Point::new(150.0, 80.0));
        let (ea, eb) = (p.piece(a).unwrap().edges()[1].id, p.piece(b).unwrap().edges()[3].id);
        p.add_seam(
            "costado",
            vec![SeamSide { piece: a, edge: ea, reversed: false }, SeamSide { piece: b, edge: eb, reversed: true }],
        );
        p
    }

    #[test]
    fn json_round_trip() {
        let p = sample();
        let json = p.to_json().unwrap();
        let back = Pattern::from_json(&json).unwrap();
        assert_eq!(p.pieces, back.pieces);
        assert_eq!(p.seams, back.seams);
    }

    #[test]
    fn arrangement_is_optional() {
        let mut p = sample();
        let json = p.to_json().unwrap();
        assert!(!json.contains("arrangement"), "la colocación por defecto no se guarda");
        p.pieces[1].arrangement = crate::Arrangement::Back;
        let back = Pattern::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back.pieces[1].arrangement, crate::Arrangement::Back);
        assert_eq!(back.pieces[0].arrangement, crate::Arrangement::Front);
    }

    #[test]
    fn avatar_fields_are_optional() {
        let mut p = sample();
        let json = p.to_json().unwrap();
        assert!(!json.contains("zone") && !json.contains("on_avatar"), "los valores por defecto no se guardan");
        p.on_avatar = true;
        p.pieces[1].zone = crate::BodyZone::LeftArm;
        let back = Pattern::from_json(&p.to_json().unwrap()).unwrap();
        assert!(back.on_avatar);
        assert_eq!(back.pieces[1].zone, crate::BodyZone::LeftArm);
        assert_eq!(back.pieces[0].zone, crate::BodyZone::Torso);
    }

    #[test]
    fn next_id_never_collides_after_load() {
        let mut p = sample();
        let json = p.to_json().unwrap().replace("\"next_id\"", "\"ignored\"");
        let mut back = Pattern::from_json(&json).unwrap();
        assert_eq!(back.alloc_id(), p.alloc_id());
    }

    #[test]
    fn rejects_other_formats_and_newer_versions() {
        let json = sample().to_json().unwrap();
        let other = json.replace(FORMAT_ID, "otra-cosa");
        assert!(matches!(Pattern::from_json(&other), Err(PatternIoError::WrongFormat(_))));
        let newer = json.replace("\"version\": 1", "\"version\": 99");
        assert!(matches!(Pattern::from_json(&newer), Err(PatternIoError::UnsupportedVersion(99))));
    }

    #[test]
    fn rejects_inconsistent_piece() {
        let json = r#"{"format":"clothing-maker/pattern","version":1,"pattern":{"name":"x","pieces":[
            {"id":1,"name":"a","vertices":[{"x":0,"y":0}],"edges":[]}]}}"#;
        assert!(Pattern::from_json(json).is_err());
    }
}
