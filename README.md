# clothing-maker

Herramienta de escritorio en Rust para diseñar patrones 2D y ver cómo quedan
en 3D con física de tela. Sirve para ropa y también para objetos sin avatar,
como mochilas o bolsas.

## Estado

**Fase 1:** editor 2D de patrones.

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

**Fase 2 (esta):** malla y tela suelta en 3D.

- Panel «Vista 3D» a la derecha del editor (Ver → Vista 3D para ocultarlo).
- Cada pieza se triangula y se convierte en tela simulada con XPBD
  (estiramiento, flexión, gravedad y rozamiento con el aire), en un hilo aparte.
- ▶ Simular / ⏸ Pausa y ⟲ Reiniciar (vuelve a colocar las piezas con el
  patrón actual; si editas el patrón, la vista avisa de que hay cambios).
- Colocación: *Soltar* (las piezas caen en horizontal) o *Colgar* (cuelgan de
  su borde superior).
- Obstáculos: solo suelo, esfera o mesa. Son colisionadores genéricos; el
  avatar será uno más, así que también sirve para objetos sin cuerpo.
- Tejidos: algodón, seda y vaquero. En ⚙, tamaño de los triángulos y
  subpasos (más subpasos = tela menos elástica, pero más lenta).
- Cámara: arrastrar para girar, botón derecho / central (o Mayús + arrastrar)
  para desplazar, rueda para acercar, doble clic para volver a encuadrar.

Aún no hay costuras en 3D ni colisión de la tela consigo misma: las piezas
caen sueltas y una pieza puede atravesarse a sí misma al plegarse.

**Siguientes fases:** costuras de 2 o más tramos en el editor y cerradas en 3D,
avatar con puntos de colocación, remallado en vivo y auto-colisión, y después
tejidos, GPU y exportación.

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
| `crates/simulator` | Simulación de tela XPBD (`glam`, `rayon`): todas las piezas en un sistema de partículas, restricciones coloreadas para resolverlas en paralelo, colisionadores genéricos (plano, esfera, caja). Sin dependencias gráficas. |
| `crates/app` | Aplicación de escritorio con `eframe`/`egui` sobre `wgpu`; la vista 3D se dibuja con `wgpu` en una textura propia (`src/view3d`). |

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
