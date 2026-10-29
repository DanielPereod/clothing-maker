//! Biblioteca de telas de un patrón.
//!
//! Cada [`Piece`](crate::Piece) dice de qué tela es y qué capas lleva
//! debajo (forro, entretela...), apuntando a telas de la biblioteca del
//! [`Pattern`](crate::Pattern) por su [`TextileId`]. Así, al ajustar la
//! firmeza del cuero cambian todas las piezas de cuero.
//!
//! Las propiedades van en unidades fáciles de ajustar a mano (escalas de 0 a
//! 100, gramos por metro cuadrado, milímetros); la vista 3D las traduce a
//! física.

use serde::{Deserialize, Serialize};

/// Identificador de una tela dentro de la biblioteca de un patrón.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextileId(pub u32);

impl TextileId {
    /// El algodón de la biblioteca genérica: la tela de las piezas que no
    /// dicen otra cosa.
    pub const COTTON: TextileId = TextileId(1);

    pub(crate) fn is_cotton(&self) -> bool {
        *self == TextileId::COTTON
    }
}

impl Default for TextileId {
    fn default() -> Self {
        TextileId::COTTON
    }
}

/// Una tela de la biblioteca.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Textile {
    pub id: TextileId,
    pub name: String,
    /// Firmeza al doblar, de 0 (cae como un velo) a 100 (cuero de bolso).
    /// Es logarítmica: cada 25 puntos, diez veces más firme.
    pub firmness: f64,
    /// Gramaje (g/m²). Cuanto más pesa, más cae.
    pub weight: f64,
    /// Elasticidad, de 0 (no cede nada) a 100 (muy elástica). También
    /// logarítmica.
    pub stretch: f64,
    /// Rozamiento contra el cuerpo o los obstáculos (0..1).
    pub friction: f64,
    /// Grosor (mm).
    pub thickness: f64,
}

/// Rangos de los ajustes, para la interfaz.
pub const FIRMNESS_RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
pub const WEIGHT_RANGE: std::ops::RangeInclusive<f64> = 20.0..=1500.0;
pub const STRETCH_RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
pub const FRICTION_RANGE: std::ops::RangeInclusive<f64> = 0.0..=1.0;
pub const THICKNESS_RANGE: std::ops::RangeInclusive<f64> = 0.5..=6.0;

impl Textile {
    fn generic(id: u32, name: &str, firmness: f64, weight: f64, stretch: f64, friction: f64, thickness: f64) -> Self {
        Self { id: TextileId(id), name: name.into(), firmness, weight, stretch, friction, thickness }
    }

    /// La biblioteca genérica con la que empieza cada patrón.
    pub fn library() -> Vec<Textile> {
        vec![
            Textile::generic(1, "Algodón", 42.0, 150.0, 50.0, 0.5, 3.0),
            Textile::generic(2, "Lino", 48.0, 180.0, 40.0, 0.5, 3.0),
            Textile::generic(3, "Seda", 22.0, 60.0, 57.0, 0.2, 2.0),
            Textile::generic(4, "Punto", 35.0, 180.0, 75.0, 0.5, 3.0),
            Textile::generic(5, "Vaquero", 67.0, 400.0, 32.0, 0.6, 4.0),
            Textile::generic(6, "Lona", 75.0, 350.0, 30.0, 0.6, 4.0),
            Textile::generic(7, "Fieltro", 70.0, 300.0, 45.0, 0.7, 4.0),
            Textile::generic(8, "Cuero", 95.0, 900.0, 25.0, 0.7, 2.0),
            Textile::generic(9, "Entretela", 70.0, 50.0, 35.0, 0.4, 1.0),
            Textile::generic(10, "Forro", 20.0, 70.0, 55.0, 0.2, 1.0),
        ]
    }

    /// La versión genérica de esta tela, si la hay (para volver a sus
    /// valores de fábrica).
    pub fn generic_version(&self) -> Option<Textile> {
        Textile::library().into_iter().find(|t| t.id == self.id)
    }
}

impl Default for Textile {
    fn default() -> Self {
        Textile::library().swap_remove(0)
    }
}

pub(crate) fn is_generic_library(library: &Vec<Textile>) -> bool {
    *library == Textile::library()
}
