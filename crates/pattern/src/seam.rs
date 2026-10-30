//! Cómo se hace una costura: tipo, hacia dónde se plancha el margen y
//! pespuntes vistos.

use serde::{Deserialize, Serialize};

/// Margen de costura (mm) que se usa si el patrón no dice otro.
pub const DEFAULT_SEAM_ALLOWANCE: f64 = 10.0;

/// Largo de puntada (mm) por defecto.
pub const DEFAULT_STITCH_LENGTH: f64 = 2.5;

/// Tipo de costura. Decide dónde queda el margen (dentro o fuera de la
/// prenda), cuántas capas de tela se juntan en ella y cómo se dobla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamKind {
    /// Derecho con derecho y el margen por dentro, abierto o planchado a un
    /// lado. La más habitual.
    #[default]
    Plain,
    /// Como la simple, con los dos márgenes rematados juntos con remalladora
    /// y planchados a un lado (camisetas, punto).
    Overlocked,
    /// Revés con revés: el margen queda por fuera, a la vista, como adorno.
    Exposed,
    /// Costura dentro de otra: se cose revés con revés, se recorta, se da la
    /// vuelta y se cose otra vez encerrando el margen. Sin cantos a la vista
    /// (telas finas, lencería).
    French,
    /// Inglesa o plana (la de los vaqueros): un margen envuelve al otro y se
    /// pespuntea plano por fuera con dos pespuntes. Muy resistente y plana.
    FlatFelled,
    /// Solapada: una pieza encima de la otra y pespunteada por fuera.
    Lapped,
    /// Embolsada o volteada: se cose derecho con derecho y se da la vuelta,
    /// así la costura queda en el borde y el margen escondido entre las dos
    /// capas (cuellos, vistas, carteras).
    Bagged,
}

/// Hacia dónde tiende a doblarse la tela en la costura.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamFold {
    /// Sin preferencia: la costura hace de bisagra.
    Free,
    /// Las piezas siguen planas una a continuación de la otra.
    Flat,
    /// Las piezas se doblan una sobre otra (borde de una costura volteada).
    Folded,
}

/// Dónde queda el margen respecto a la tela.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllowanceSide {
    /// Por el revés de la tela (dentro de la prenda).
    WrongSide,
    /// Por el derecho (fuera, a la vista).
    RightSide,
}

impl SeamKind {
    pub const ALL: [SeamKind; 7] = [
        SeamKind::Plain,
        SeamKind::Overlocked,
        SeamKind::Exposed,
        SeamKind::French,
        SeamKind::FlatFelled,
        SeamKind::Lapped,
        SeamKind::Bagged,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SeamKind::Plain => "Simple",
            SeamKind::Overlocked => "Sobrehilada",
            SeamKind::Exposed => "A la vista",
            SeamKind::French => "Francesa",
            SeamKind::FlatFelled => "Inglesa (plana)",
            SeamKind::Lapped => "Solapada",
            SeamKind::Bagged => "Volteada",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SeamKind::Plain => {
                "Derecho con derecho, el margen queda por dentro, abierto o planchado a un lado. La más habitual."
            }
            SeamKind::Overlocked => {
                "Como la simple, con los dos márgenes rematados juntos (remalladora) y planchados a un lado. \
                 Típica de camisetas y tejidos de punto."
            }
            SeamKind::Exposed => "Revés con revés: el margen queda por fuera, a la vista, como adorno.",
            SeamKind::French => {
                "Una costura dentro de otra que encierra el margen: por dentro no se ve ningún canto. \
                 Para telas finas y transparentes."
            }
            SeamKind::FlatFelled => {
                "La de los vaqueros: un margen envuelve al otro y se cose plano por fuera con dos pespuntes. \
                 Muy resistente y plana."
            }
            SeamKind::Lapped => "Una pieza se pone encima de la otra y se pespuntea por fuera. No se dobla.",
            SeamKind::Bagged => {
                "Se cose derecho con derecho y se da la vuelta: la costura queda justo en el borde, con el margen \
                 escondido entre las dos capas. Cuellos, vistas, carteras."
            }
        }
    }

    /// Margen (mm) que se suele dejar para este tipo de costura.
    pub fn typical_allowance(self) -> f64 {
        match self {
            SeamKind::Plain | SeamKind::Exposed | SeamKind::Lapped | SeamKind::Bagged => 10.0,
            SeamKind::Overlocked => 7.0,
            SeamKind::French => 15.0,
            SeamKind::FlatFelled => 15.0,
        }
    }

    /// Capas de tela de margen que se apilan en la costura, contando todos
    /// los lados (en una de dos piezas).
    pub fn layers(self) -> f32 {
        match self {
            SeamKind::Plain | SeamKind::Exposed | SeamKind::Bagged | SeamKind::Lapped => 2.0,
            SeamKind::Overlocked => 2.5,
            SeamKind::French => 4.0,
            SeamKind::FlatFelled => 4.0,
        }
    }

    /// Cuánto más rígida que la tela es la costura al doblarla.
    pub fn stiffness(self) -> f32 {
        match self {
            SeamKind::Plain | SeamKind::Exposed => 1.0,
            SeamKind::Overlocked => 2.0,
            SeamKind::Bagged => 2.0,
            SeamKind::French => 3.0,
            SeamKind::Lapped => 3.0,
            SeamKind::FlatFelled => 5.0,
        }
    }

    /// Hacia dónde tiende a doblarse la costura.
    pub fn fold(self) -> SeamFold {
        match self {
            SeamKind::Plain | SeamKind::Overlocked | SeamKind::Exposed => SeamFold::Free,
            SeamKind::French | SeamKind::FlatFelled | SeamKind::Lapped => SeamFold::Flat,
            SeamKind::Bagged => SeamFold::Folded,
        }
    }

    pub fn allowance_side(self) -> AllowanceSide {
        match self {
            SeamKind::Exposed => AllowanceSide::RightSide,
            _ => AllowanceSide::WrongSide,
        }
    }

    /// Qué parte del margen se ve una vez hecha la costura (la francesa y la
    /// inglesa lo doblan y encierran).
    pub fn visible_allowance(self) -> f64 {
        match self {
            SeamKind::French => 0.4,
            SeamKind::FlatFelled => 0.5,
            _ => 1.0,
        }
    }

    /// Si el margen se plancha a un lado aunque no se diga (los dos márgenes
    /// van juntos).
    pub fn pressed_together(self) -> bool {
        matches!(self, SeamKind::Overlocked | SeamKind::French | SeamKind::FlatFelled | SeamKind::Lapped)
    }

    /// Pespuntes vistos que lleva normalmente.
    pub fn typical_topstitch(self) -> Topstitch {
        match self {
            SeamKind::FlatFelled => Topstitch::Double,
            SeamKind::Lapped => Topstitch::Single,
            _ => Topstitch::None,
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        *self == SeamKind::default()
    }
}

/// Pespuntes vistos por el derecho, paralelos a la costura.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topstitch {
    #[default]
    None,
    Single,
    Double,
}

impl Topstitch {
    pub const ALL: [Topstitch; 3] = [Topstitch::None, Topstitch::Single, Topstitch::Double];

    pub fn label(self) -> &'static str {
        match self {
            Topstitch::None => "Sin pespunte",
            Topstitch::Single => "Un pespunte",
            Topstitch::Double => "Doble pespunte",
        }
    }

    /// Número de líneas de pespunte.
    pub fn lines(self) -> usize {
        match self {
            Topstitch::None => 0,
            Topstitch::Single => 1,
            Topstitch::Double => 2,
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        *self == Topstitch::default()
    }
}

/// Cómo casan los lados de una costura que no miden lo mismo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamFit {
    /// Los lados miden lo mismo; si no, el patrón avisa.
    #[default]
    Exact,
    /// El lado largo se embebe: algo más largo, pero sin pliegues (una
    /// manga en la sisa).
    Eased,
    /// El lado largo se frunce hasta el largo del corto (una falda en la
    /// cinturilla, la boca de un saco).
    Gathered,
    /// El lado corto, de punto o elástico, se cose estirado hasta el largo
    /// del otro (un puño, una cinturilla de punto).
    Stretched,
}

impl SeamFit {
    pub const ALL: [SeamFit; 4] = [SeamFit::Exact, SeamFit::Eased, SeamFit::Gathered, SeamFit::Stretched];

    pub fn label(self) -> &'static str {
        match self {
            SeamFit::Exact => "Igual",
            SeamFit::Eased => "Embebida",
            SeamFit::Gathered => "Fruncida",
            SeamFit::Stretched => "Estirada",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SeamFit::Exact => "Los lados miden lo mismo",
            SeamFit::Eased => "El lado largo se embebe, sin pliegues (hasta un 10 % más)",
            SeamFit::Gathered => "El lado largo se frunce hasta el largo del corto",
            SeamFit::Stretched => "El lado corto (punto, elástico) se cose estirado",
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        *self == SeamFit::default()
    }
}
