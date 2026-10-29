//! Deshacer / rehacer por instantáneas del patrón.
//!
//! En vez de registrar cada operación, al final de cada fotograma en el que
//! no hay ninguna interacción en curso se compara el patrón con la última
//! versión confirmada. Así un arrastre largo cuenta como un solo paso.

use pattern::Pattern;

const MAX_STEPS: usize = 200;

pub struct History {
    undo: Vec<Pattern>,
    redo: Vec<Pattern>,
    committed: Pattern,
}

impl History {
    pub fn new(pattern: &Pattern) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), committed: pattern.clone() }
    }

    /// Olvida el historial (al abrir o crear un patrón).
    pub fn reset(&mut self, pattern: &Pattern) {
        *self = Self::new(pattern);
    }

    /// Confirma el estado actual como un paso del historial si ha cambiado.
    pub fn commit(&mut self, current: &Pattern) {
        if *current != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, current.clone()));
            self.redo.clear();
            if self.undo.len() > MAX_STEPS {
                self.undo.remove(0);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, current: &mut Pattern) -> bool {
        self.commit(current);
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.committed, prev.clone()));
        *current = prev;
        true
    }

    pub fn redo(&mut self, current: &mut Pattern) -> bool {
        self.commit(current);
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.committed, next.clone()));
        *current = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::Point;

    #[test]
    fn undo_redo_round_trip() {
        let mut p = Pattern::new("t");
        let mut h = History::new(&p);
        p.add_rect("a", Point::ZERO, 10.0, 10.0);
        h.commit(&p);
        p.add_rect("b", Point::ZERO, 10.0, 10.0);
        // Sin confirmar: deshacer confirma primero y luego vuelve atrás.
        assert!(h.undo(&mut p));
        assert_eq!(p.pieces.len(), 1);
        assert!(h.undo(&mut p));
        assert_eq!(p.pieces.len(), 0);
        assert!(!h.undo(&mut p));
        assert!(h.redo(&mut p));
        assert!(h.redo(&mut p));
        assert_eq!(p.pieces.len(), 2);
        assert!(!h.redo(&mut p));
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut p = Pattern::new("t");
        let mut h = History::new(&p);
        p.add_rect("a", Point::ZERO, 10.0, 10.0);
        h.undo(&mut p);
        assert!(h.can_redo());
        p.name = "otro".into();
        h.commit(&p);
        assert!(!h.can_redo());
    }
}
