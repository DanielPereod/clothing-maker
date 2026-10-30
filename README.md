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
- Ejemplos: camiseta básica, camiseta con mangas, bolsa tote, bolsa con fondo
  y cojín (Archivo → Ejemplos).

Navegación: rueda para zoom, arrastrar en vacío / botón central / derecho para
desplazar, F para encuadrar todo.

**Fase 2:** malla y tela suelta en 3D.

- Vista 3D a la derecha del editor (los botones de disposición, arriba a la
  derecha, muestran solo el patrón, los dos o solo la vista 3D).
- Cada pieza se triangula y se convierte en tela simulada con XPBD
  (estiramiento, flexión, gravedad y rozamiento con el aire), en un hilo aparte.
- ▶ Simular / ⏸ Pausa y ⟲ Reiniciar (vuelve a colocar las piezas con el
  patrón actual; si editas el patrón, la vista avisa de que hay cambios).
- Colocación: *Alrededor* (ver fase 3), *Soltar* (las piezas caen en
  horizontal) o *Colgar* (cuelgan de su borde superior).
- Obstáculos: maniquí (fase 4), solo suelo, esfera o mesa. Son colisionadores
  genéricos, así que también sirve para objetos sin cuerpo.
- Tejidos: algodón, seda y vaquero. En *Precisión*, tamaño de los triángulos y
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
- *Colocación 3D* de cada pieza (en la etapa *Colocación*): delante, detrás,
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

**Fase 5:** cambios en vivo y auto-colisión.

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
- Auto-colisión (en *Precisión*, activada por defecto): las partículas no se acercan a
  menos de dos grosores de los triángulos de la propia tela, así que los
  paneles no se atraviesan: las dos caras de una bolsa quedan una encima de
  otra, una manga no entra en el cuerpo. Hash espacial y prueba punto contra
  triángulo; cada partícula recuerda de qué lado estaba y vuelve a él si se
  cuela. Encarece el paso (de unos 6 a 12 ms con la camiseta); se puede
  apagar.

**Fase 6 (esta):** relleno (presión interior).

- *Relleno* en las propiedades del patrón (en pascales, se guarda en el
  fichero): la prenda se hincha como si tuviera algo dentro. Con 0, una bolsa
  vacía queda plana al apoyarla, como una de verdad. Con unos 10 Pa una bolsa
  se llena; con 30, un cojín queda firme. Mucha más presión la pone redonda
  como un globo.
- Cada triángulo se empuja hacia fuera (el derecho de la tela) con
  `presión × área`. Las bocas sin coser (la de una bolsa, el cuello de una
  camiseta) se cierran con una tapa imaginaria que también empuja, así que la
  boca se abre en vez de vaciarse y la suma de fuerzas es cero: la prenda no
  sale disparada.
- La presión empieza cuando la prenda, ya cosida, se ha parado, y sube en un
  segundo: si se hinchara mientras cae, llegaría redonda y rodaría. Cambiar
  el relleno en vivo no la deshincha.
- La vista 3D muestra la presión aplicada y el volumen que encierra la tela.
- Ejemplo nuevo: *Cojín* (dos cuadrados de 40 cm cosidos por los cuatro
  lados, con 30 Pa).

**Costuras:** etapa *Montaje* dedicada a cómo se hace cada costura.

- Tipos: simple, sobrehilada (remallada), a la vista, francesa, inglesa
  (plana, la de los vaqueros), solapada y volteada (embolsada). Cada tipo
  dice dónde queda el margen, cuántas capas junta y cómo se dobla; al
  elegirlo se ponen los pespuntes y el planchado que suele llevar.
- Por costura: hacia qué pieza va planchado el margen (o abierto), pespuntes
  vistos (ninguno, uno o dos), largo de puntada y orden de montaje (▲ / ▼).
  Las propiedades muestran un dibujo de la **sección** de la costura.
- **Margen de costura** por borde (10 mm por defecto en el patrón, editable
  en cada arista o desde la costura). En 2D se dibuja la línea de corte
  discontinua alrededor de cada pieza.
- **Derecho y revés:** cada pieza puede tener el *derecho hacia dentro*
  (forros, vistas) y la prenda entera se puede **voltear** (del revés). En 3D
  el derecho se ve del color de la pieza y el revés más claro y apagado.
- En el montaje, el patrón se atenúa y las costuras destacan; la
  seleccionada dice su tipo y une con líneas finas los puntos que se cosen
  juntos (si se cruzan, algún lado va al revés).
- En 3D: el margen se ve tumbado por su cara (dentro o, en la costura a la
  vista y en la prenda volteada, fuera), con el remallado en las
  sobrehiladas; los pespuntes se dibujan puntada a puntada; *Resaltar* marca
  cada costura con su color.
- En la simulación, cada tipo cambia la costura: más rígida a lo largo,
  más gruesa y pesada cuantas más capas junta (francesa, inglesa), y con un
  pliegue preferido: plana (francesa, inglesa, solapada o con pespunte) o
  doblada una pieza sobre otra (volteada). *Coser por orden* cierra las
  costuras de una en una en el orden de la lista.

**Flujo por etapas:** como en Fusion 360 (primero el boceto, luego el
volumen), una prenda se hace en orden y cada etapa tiene sus herramientas,
sus propiedades y sus comprobaciones:

1. **Patrón**: dibujar las piezas (pluma, rectángulo, curvas, puntos).
2. **Telas**: la tela de cada pieza y las capas pegadas (entretelas,
   refuerzos); el lienzo pinta cada pieza del color de su tela y la
   biblioteca de telas está a mano.
3. **Montaje**: las costuras, su tipo, el margen, el orden y voltear.
4. **Colocación**: dónde empieza cada pieza en 3D antes de coser (lado, zona
   del cuerpo, cara), el modo de colocación y el maniquí. *Colocación* en la
   cinta vuelve a colocar las piezas y para, para verlas antes de coser.
5. **Simulación**: coser y asentar la prenda (obstáculo, relleno, precisión).

- El lienzo solo deja hacer lo de la etapa: la forma de las piezas se
  cambia en *Patrón*, las aristas se eligen en *Patrón* y *Montaje*, y las
  piezas se pueden mover también en *Colocación* (su sitio en el patrón
  cuenta al colocarlas). Las herramientas de dibujo (P, R) llevan a
  *Patrón* y la de coser (S), a *Montaje*.
- **Comprobaciones**: cada pestaña dice cuántas cosas tiene pendientes y el
  navegador las lista por etapa (clic para ir y seleccionarlo): piezas sin
  área o que se cruzan, telas que faltan, costuras rotas o de largos
  distintos, piezas sin coser, piezas que se solapan en el mismo sitio de
  colocación (en 3D se cruzarían), costuras que se retuercen al colocar las
  piezas (un lado va al revés) y piezas que no se pueden mallar.

**Líneas interiores, agujeros, espejo y corte al doblez:** lo que hace
falta para piezas de verdad (bolsillos, ventanas, piezas simétricas).

- Herramienta *Línea* (I): arrastra dentro de una pieza para dibujar una
  línea interior (dónde va un bolsillo, un asa, un cierre). Se puede coser
  como cualquier arista, así que una pieza se cose *encima* de otra; en 3D
  la malla sigue la línea y la costura tira de esos puntos. Sus extremos se
  mueven arrastrándolos y sus coordenadas se editan en Propiedades.
- *Agujero*: una pieza dibujada dentro de otra pasa a ser un hueco en ella
  (una ventana, el paso de un cordón, un ojal grande). Se mueve con su
  pieza, no lleva tela y sus bordes se pueden coser (una vista o un vivo
  alrededor de la ventana). Se elige en la cinta o en *Agujero en* de
  Propiedades.
- *Espejo* crea la pieza simétrica (izquierda y derecha) con sus líneas,
  y *Desplegar al doblez* (menú de la arista) convierte media pieza cortada
  al doblez en la pieza entera, uniendo las dos mitades por esa arista.
- Comprobaciones nuevas: líneas fuera de su pieza o que se cruzan y agujeros
  que se salen de la pieza. Una pieza cosida sobre una línea de otra puede
  solaparse con ella sin aviso.

**Interfaz:** inspirada en los programas de CAD como Fusion 360, con tema
oscuro.

- Barra superior: menús, accesos rápidos (nuevo, abrir, guardar, deshacer,
  rehacer), el documento abierto (con un punto si hay cambios sin guardar) y
  la disposición de las vistas.
- Cinta de herramientas con las cinco etapas numeradas y *Siguiente* para
  pasar a la próxima. *Patrón*: seleccionar, crear (pluma, rectángulo),
  modificar, inspeccionar (rejilla, malla) y encuadrar. *Telas*: tela de la
  selección (o de todas), añadir capa y tela nueva. *Montaje*: costura, tipo,
  margen, orden, voltear y resaltar. *Colocación*: lado, zona y cara de la
  pieza, maniquí, modo de colocación y coser. *Simulación*: simular / pausa,
  reiniciar, en vivo, entorno (obstáculo), precisión, inicio de cámara y
  relleno. Los grupos con ⏷ despliegan más órdenes.
- Navegador a la izquierda: las comprobaciones de cada etapa y el árbol con
  las piezas (con su tela o su sitio de colocación, según la etapa) y las
  costuras (clic para seleccionar, botón derecho para duplicar o borrar).
- Propiedades a la derecha: lo seleccionado (pieza, punto, arista o costura)
  y el patrón, con lo que importa en la etapa activa.
- Vista 3D con cubo de vista: pulsar una cara (frente, detrás, lados, arriba)
  lleva la cámara a esa vista. Cada vista tiene abajo una barra de navegación
  (encuadrar, zoom, rejilla y malla en 2D; inicio, simular, reiniciar y en
  vivo en 3D).
- Barra de estado: pista de la herramienta, estado de la simulación,
  coordenadas del cursor y zoom.

**Siguientes fases:** más tejidos, GPU y exportación.

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
| `crates/app` | Aplicación de escritorio con `eframe`/`egui` sobre `wgpu`; la vista 3D se dibuja con `wgpu` en una textura propia (`src/view3d`). Tema, iconos vectoriales y controles propios en `theme.rs`, `icons.rs` y `widgets.rs`. |

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
`right`, `top` o `bottom`; si falta, `front`). El patrón puede llevar
`"pressure"` (relleno en Pa; si falta, 0).

Costuras y márgenes (todo opcional; si falta, lo de antes): cada costura
puede llevar `"kind"` (`plain`, `overlocked`, `exposed`, `french`,
`flat_felled`, `lapped` o `bagged`), `"pressed_to"` (índice del lado hacia el
que va el margen), `"topstitch"` (`none`, `single` o `double`) y
`"stitch_length"` (mm, 2,5 por defecto). El orden de `seams` es el de
montaje. Cada arista puede llevar `"allowance"` (margen en mm); el patrón,
`"seam_allowance"` (10 por defecto) e `"inside_out"`, y cada pieza
`"right_side_in"`.

Líneas interiores y agujeros (también opcionales): cada pieza puede llevar
`"lines"`, una lista de líneas interiores
`{ "id": 30, "from": { "x": 110, "y": 120 }, "to": { "x": 110, "y": 300 }, "type": "line" }`
(o `"type": "cubic"` con `h1` y `h2`, relativos a cada extremo). Su `id` es
un id de arista más, así que un lado de costura puede apuntar a él. Una
pieza con `"hole_in": <id de pieza>` es un agujero en esa pieza.

## Desarrollo

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
