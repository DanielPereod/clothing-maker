# clothing-maker

Herramienta de escritorio en Rust para diseñar patrones 2D y ver cómo quedan
en 3D con física de tela. Sirve para ropa y también para objetos sin avatar,
como mochilas o bolsas.

## Estado

**Fase 1 (esta):** editor 2D de patrones.

- Piezas con aristas rectas y curvas Bézier cúbicas.
- Herramientas: seleccionar (V), pluma (P) y rectángulo (R).
- Mover puntos, manejadores y piezas; ajuste a rejilla (mantén Alt para desactivarlo).
- Doble clic en una arista para añadir un punto; C / L convierten la arista seleccionada en curva / recta.
- Duplicar (Ctrl+D) y borrar (Supr), deshacer / rehacer (Ctrl+Z / Ctrl+Mayús+Z).
- Longitud de cada arista en la pieza seleccionada, área y perímetro.
- Guardar y abrir en JSON (`*.pattern.json`), con aviso de cambios sin guardar.
- Vista previa de la malla de simulación y de las costuras (de momento solo
  se ven; se crean desde los ejemplos o editando el JSON).
- Ejemplos: camiseta básica y bolsa tote (Archivo → Ejemplos).

Navegación: rueda para zoom, arrastrar en vacío / botón central / derecho para
desplazar, F para encuadrar todo.

**Siguientes fases:** edición de costuras (incluidas las de más de dos piezas y
por tramos), vista 3D con `wgpu`, simulación XPBD en tiempo real con `rayon`,
colisiones y avatar opcional.

## Cómo ejecutarlo

Necesitas Rust (estable, 1.88 o superior): <https://rustup.rs>.

```sh
cargo run --release -p app
# o abrir un patrón directamente
cargo run --release -p app -- ruta/al/patron.pattern.json
```

La primera compilación tarda unos minutos (wgpu es grande); las siguientes son rápidas.

## Estructura

| Crate | Qué hace |
| --- | --- |
| `crates/pattern` | Modelo de datos: piezas, aristas (rectas / Bézier) y costuras. Formato JSON versionado. Sin dependencias gráficas. |
| `crates/mesher` | Triangula cada pieza (Delaunay restringida con `spade`) y remuestrea el contorno para que las costuras casen vértice a vértice. |
| `crates/simulator` | Simulación de tela XPBD (`glam`). De momento, el esqueleto con restricciones de distancia y gravedad. |
| `crates/app` | Aplicación de escritorio con `eframe`/`egui` sobre `wgpu`. |

Unidades: el patrón se guarda en milímetros (eje Y hacia abajo); la interfaz
muestra centímetros. El simulador trabaja en metros.

## Formato de fichero

```json
{
  "format": "clothing-maker/pattern",
  "version": 1,
  "pattern": {
    "name": "Camiseta básica",
    "pieces": [
      {
        "id": 1, "name": "Delantero",
        "vertices": [{ "x": 180, "y": 0 }, ...],
        "edges": [
          { "id": 2, "type": "cubic", "h1": { "x": 5, "y": 95 }, "h2": { "x": -5, "y": 95 } },
          { "id": 3, "type": "line" }, ...
        ]
      }
    ],
    "seams": [
      { "id": 20, "name": "Hombro", "sides": [{ "piece": 1, "edge": 3 }, { "piece": 10, "edge": 12, "reversed": true }] }
    ],
    "next_id": 21
  }
}
```

La arista `i` va del vértice `i` al `i + 1` (la última cierra el contorno).
Los manejadores de las curvas son relativos a su vértice, así que al mover un
punto la curva le acompaña. Las costuras apuntan a ids de arista, que no
cambian al añadir o quitar puntos en otras partes de la pieza.

## Desarrollo

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
