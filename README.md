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
- Vista previa de la malla de simulación.
- Ejemplos: camiseta básica, camiseta con mangas, bolsa tote y bolsa con fondo
  (Archivo → Ejemplos).

Navegación: rueda para zoom, arrastrar en vacío / botón central / derecho para
desplazar, F para encuadrar todo.

**Fase 2:** malla y tela suelta en 3D.

- Panel «Vista 3D» a la derecha del editor (Ver → Vista 3D para ocultarlo).
- Cada pieza se triangula y se convierte en tela simulada con XPBD
  (estiramiento, flexión, gravedad y rozamiento con el aire), en un hilo aparte.
- ▶ Simular / ⏸ Pausa y ⟲ Reiniciar (vuelve a colocar las piezas con el
  patrón actual; si editas el patrón, la vista avisa de que hay cambios).
- Colocación: *Alrededor* (ver fase 3), *Soltar* (las piezas caen en
  horizontal) o *Colgar* (cuelgan de su borde superior).
- Obstáculos: maniquí (fase 4), solo suelo, esfera o mesa. Son colisionadores
  genéricos, así que también sirve para objetos sin cuerpo.
- Tejidos: algodón, seda y vaquero. En ⚙, tamaño de los triángulos y
  subpasos (más subpasos = tela menos elástica, pero más lenta).
- Cámara: arrastrar para girar, botón derecho / central (o Mayús + arrastrar)
  para desplazar, rueda para acercar, doble clic para volver a encuadrar.

**Fase 3:** costuras.

- Herramienta *Costura* (S): haz clic en cada arista cerca del extremo por el
  que empieza la costura; con el segundo clic se crea. Mayús + clic añade más
  lados (una costura puede unir 3 o más aristas, por ejemplo tres paneles que
  se juntan en un fondo). Volver a pulsar una arista ya elegida le da la vuelta.
- Cada lado se dibuja con un trazo discontinuo, un punto donde empieza y una
  flecha con su sentido. Al seleccionar una costura (en la lista de
  Costuras o desde la arista) se puede renombrar, invertir o quitar cada
  lado, o borrarla (Supr).
- Las piezas se dibujan vistas desde fuera, con el derecho de la tela hacia
  ti: la espalda, como se vería desde detrás. Por eso, al coser dos piezas
  vecinas, lo normal es que una de las dos aristas vaya al revés; basta con
  pinchar cada arista por el extremo que corresponde.
- *Colocación 3D* de cada pieza (en sus propiedades): delante, detrás,
  izquierda, derecha, arriba o abajo. En la vista 3D, la colocación
  *Alrededor* curva las piezas sobre un cilindro, cada una en su lado y con
  la disposición del editor, y las costuras las van cerrando (casilla *Coser*).
- En la simulación, cada costura une los puntos emparejados de sus lados (el
  mallador les da el mismo número de puntos aunque midan algo distinto). La
  costura se cierra poco a poco y sin gravedad; cuando está cerrada, la pieza
  cae sobre el obstáculo. No hace falta avatar: sirve para bolsas y mochilas.

**Fase 4:** maniquí y puntos de colocación.

- Maniquí sacado de la malla base de MakeHuman (CC0, ver
  [`assets/README.md`](assets/README.md)). Es opcional: la casilla *Prenda
  para el maniquí* del patrón lo pone en la vista 3D (y lo quita para bolsas
  y mochilas); también se puede elegir a mano en la lista de obstáculos.
- La tela choca con el cuerpo mediante un campo de distancia precalculado
  (rejilla de 8 mm, menos de un segundo al abrir la app). Cualquier malla
  cerrada sirve como colisionador.
- Puntos de colocación: con el maniquí, cada pieza tiene una *Zona del
  cuerpo* (torso, brazo derecho o izquierdo, pierna derecha o izquierda) y,
  dentro de ella, su *Colocación 3D* de siempre. La colocación *Alrededor*
  curva cada pieza alrededor del eje de su zona, con algo de holgura sobre el
  cuerpo, y las costuras la cierran. En los brazos, *Delante* es la cara de
  arriba, así que una manga de una pieza cierra por debajo del brazo.
- Derecha e izquierda de las zonas son las del maniquí; la colocación
  *Derecha* / *Izquierda* de las piezas sigue siendo la de quien mira la
  prenda de frente.

**Fase 5 (esta):** cambios en vivo y auto-colisión.

- *En vivo* (activado por defecto): al editar el patrón, la prenda 3D se
  reajusta sin volver a empezar. Cada pieza se vuelve a mallar y cada punto
  nuevo toma la posición 3D que tenía ese punto del patrón en la malla vieja
  (interpolando en su triángulo). Funciona aunque cambie el tamaño de los
  triángulos, se mueva la pieza en el editor o la pieza crezca (los puntos
  nuevos siguen la forma del borde). Las piezas nuevas aparecen en su sitio
  de salida y se cosen; las costuras que cambian de largo se vuelven a
  cerrar. Mientras una pieza no se pueda mallar (un contorno que se cruza),
  la vista 3D espera. Cambiar el tejido o la malla tampoco reinicia; cambiar
  la colocación, el obstáculo o *Coser* sí, igual que *Reiniciar*.
- Auto-colisión (en ⚙, activada por defecto): las partículas no se acercan a
  menos de dos grosores de los triángulos de la propia tela, así que los
  paneles no se atraviesan: las dos caras de una bolsa quedan una encima de
  otra, una manga no entra en el cuerpo. Hash espacial y prueba punto contra
  triángulo; cada partícula recuerda de qué lado estaba y vuelve a él si se
  cuela. Encarece el paso (de unos 6 a 12 ms con la camiseta); se puede
  apagar.
- Una bolsa vacía sigue quedando plana al apoyarla, como una de verdad sin
  nada dentro; darle volumen (relleno o presión) es de la siguiente fase.

**Siguientes fases:** tejidos (con relleno o presión para objetos), GPU y
exportación.

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
| `crates/simulator` | Simulación de tela XPBD (`glam`, `rayon`): todas las piezas en un sistema de partículas, restricciones coloreadas para resolverlas en paralelo, costuras que se cierran poco a poco, colisionadores genéricos (plano, esfera, caja). Sin dependencias gráficas. |
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
cambian al añadir o quitar puntos en otras partes de la pieza. Cada lado de
una costura puede llevar `"reversed": true` (se recorre del final al
principio), y cada pieza un `"arrangement"` (`front`, `back`, `left`,
`right`, `top` o `bottom`; si falta, `front`).

## Desarrollo

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
