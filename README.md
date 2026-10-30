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
  `presión × área` (ver *Física* para las prendas con forro). Las bocas sin coser (la de una bolsa, el cuello de una
  camiseta) se cierran con una tapa imaginaria que también empuja, así que la
  boca se abre en vez de vaciarse y la suma de fuerzas es cero: la prenda no
  sale disparada.
- La presión empieza cuando la prenda, ya cosida, se ha parado, y sube en dos
  segundos: si se hinchara mientras cae, llegaría redonda y rodaría. Cambiar
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
   del cuerpo, cara, capa y si va encima de otra), el modo de colocación y
   el maniquí. *Colocación* en la
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

**Capas y colocación:** para lo que lleva varias telas cosidas entre sí
(un bolso con forro suelto, un bolsillo por dentro, una vista).

- *Capa* (Colocación): 0 es el exterior, 1 el forro y así hacia dentro.
  Cada capa se coloca aparte, dentro de la anterior (en *Alrededor*, en un
  cilindro más estrecho; soltadas o colgadas, en otro nivel), así que el
  forro empieza dentro del exterior y no hay que dibujarlo en otro sitio.
  Lo que va pegado a toda la pieza (una entretela) sigue yendo en las capas
  de su tela, en *Telas*.
- *Sobre*: una pieza puede empezar encima de otra, en su sitio y pegada a
  ella (un bolsillo de parche, un asa, un refuerzo), por su derecho o por su
  revés. Si va cosida a las líneas interiores de una sola pieza, va encima
  de ella sin decir nada.
- La auto-colisión mira también arista contra arista, además de punto
  contra triángulo: dos capas que se pliegan juntas ya no se atraviesan por
  las aristas. Una tote con forro aparte (`bolsa-tote-con-forro`) se cose y
  se asienta sin aristas cruzadas.
- Comprobaciones: las piezas de capas distintas, o encima de otra, pueden
  solaparse en el patrón sin aviso. Un consejo de patronaje: el forro suele
  cortarse un poco más pequeño que el exterior, para que no sobre tela
  dentro.

**Costuras de verdad:** lo que piden las piezas que no miden lo mismo o
que se cosen dando la vuelta (una base redonda, un puño, un volante).

- *Tramos y cadenas*: un lado de costura puede seguir el contorno por varias
  aristas seguidas (una base redonda hecha de cuatro curvas se cose en una
  sola costura) y coser solo un tramo de su largo (*Solo un tramo* en
  Propiedades, desde y hasta en cm). Con la herramienta *Costura*,
  Ctrl+clic en otra arista de la misma pieza alarga el lado hasta ella; con
  una costura seleccionada, alarga su lado. En Propiedades, + y − añaden o
  quitan la última arista.
- *Piquetes* (N, en *Montaje*): marcas en el borde para casar los lados.
  Si todos los lados de una costura tienen los mismos, la malla los hace
  coincidir: cada tramo entre piquetes se reparte por su cuenta.
- *Ajuste* de la costura: *igual* (los lados miden lo mismo), *embebida*
  (el lado largo se embebe, hasta un 10 % más), *fruncida* (el largo se
  frunce hasta el corto) o *estirada* (el corto, de punto o elástico, se
  cose estirado). Propiedades muestra la proporción largo : corto. En 3D,
  la fruncida y la embebida encogen la tela junto a la costura, cada vez
  menos al alejarse, y la tela se abre en ondas; en la estirada cede la
  tela más elástica.
- *Línea de montaje* (abajo, en *Montaje* y *Simulación*): etapas con
  nombre (Costados, Base, Puño...) que se cosen en orden: cada una empieza
  cuando están cerradas las anteriores. Se crean, renombran, ordenan y
  borran ahí, y cada costura se pasa a una etapa desde Propiedades o con
  *+ la costura elegida*. Al simular se resalta la etapa que se está
  cosiendo y la barra de estado la nombra. *Coser por orden* cose además
  las costuras de cada etapa de una en una.
- Comprobaciones: lados de largo distinto en una costura *igual* (con la
  pista de marcarla embebida, fruncida o estirada), una embebida que embebe
  demasiado y lados con piquetes distintos.
- Un tubo cosido a sí mismo (un cuerpo de una sola pieza con su costado)
  se coloca casi dando la vuelta entera, para que la costura cierre sin
  tumbarlo.
- El *Chalk bag* tiene ahora sus medidas reales: cuerpo de una pieza con su
  costado, base redonda en una sola costura con piquetes y puño de punto
  más corto que la boca, cosido estirado, en tres etapas.

**Dobleces y fornituras:** lo que lleva un bolso de verdad además de
tela: dobladillos y solapas, cremallera, asas de cincha, bies, vivos,
elásticos, broches y presillas.

- *Doblez* (D, en *Patrón*): arrastra de un borde de la pieza a otro y la
  tela se dobla por esa línea; la parte pequeña es la solapa (*Cambiar
  solapa* la cambia). En Propiedades: el ángulo (0 a 180°), hacia qué cara
  se dobla y si va *planchado* (empieza ya doblado en 3D y se queda así,
  como un dobladillo) o es una bisagra libre (una tapa que cae con su
  peso). La malla parte la pieza por la línea y en 3D se pliega por ahí.
- *Fornituras* (en *Montaje*): se generan como piezas y costuras normales,
  con su tela (cinta de cremallera, cincha, bies, vivo, elástico), así que
  se ven, se editan y se simulan como lo demás.
  - *Cremallera*: con una costura de dos lados elegida, la sustituye por
    dos cintas cosidas a cada lado y los dientes entre ellas (una costura
    de tipo *cremallera*); sin nada elegido, crea una suelta. En
    Propiedades, *Abierta* (0 a 100 %) la abre desde el tirador: en vivo se
    ve abrirse y cerrarse.
  - *Cincha*: una cinta del largo y ancho que digas, para asas y tirantes;
    se cose a líneas interiores como cualquier otra pieza y empieza en arco,
    fuera del panel.
  - *Bies* (con una arista elegida): una tira del doble de ancho, doblada
    por la mitad, que envuelve el canto.
  - *Vivo* (con una costura elegida): una tira doblada que asoma entre los
    lados de la costura.
  - *Elástico* (con una arista elegida): una cinta un 20 % más corta cosida
    a lo largo del borde; la tela se frunce hasta su largo, como al soltar
    un elástico cosido estirado.
- *Broche* (B, en *Montaje*): clic en un punto de la tela y luego en el
  punto con el que se une. Puede ser un *broche* (se abre y se cierra, en
  vivo también) o una *presilla* (unas puntadas que fijan los dos puntos
  para siempre: el forro al exterior, el extremo de un asa). Se unen al
  final del montaje, como al rematar la prenda.
- Ejemplo nuevo: *Bolso con cremallera y asas* (`bolso-con-cremallera`):
  dos paneles de lona, una cremallera en la boca y dos asas de cincha, en
  tres etapas (asas, cremallera, bolsa).

**Física:** cada tela se comporta como la de verdad.

- Cada tela estira lo que le toca: la lona y el cuero casi nada, el punto
  bastante y el elástico mucho. Antes el paso de tiempo ponía un tope a la
  rigidez y todas cedían parecido.
- Las telas tejidas llevan *tirantes*: uniones entre puntos lejanos de la
  misma pieza que no la dejan alargarse más de un 1 o 2 %, pese lo que pese
  lo que cuelga y sea como sea de fina la malla. Un asa que sostiene el
  bolso ya no se alarga un 30 %. Solo frenan al estirar: la tela se dobla y
  se arruga igual.
- En una costura *estirada*, cada lado cede según lo que estira su tela: el
  puño de punto del chalk bag (341 mm) se estira hasta los 381 mm de la boca
  y la boca de lona no se encoge para salirle al encuentro.
- Las costuras dan rigidez y grosor también en telas firmes (lona, cuero,
  vaquero), y en ellas la tela decide sola hacia dónde se tumba la costura.
  Ningún ejemplo deja aristas cruzadas, tampoco el bolso de cuero.
- El relleno va por compartimentos: empuja la tela que forma la prenda (en
  una bolsa con forro, el forro hacia el exterior, sin hinchar el hueco
  entre los dos), y no las piezas montadas encima (un bolsillo) ni las que
  casi no van cosidas. Empieza medio segundo después de que la prenda,
  cosida, se haya parado.
- *Fijar* (en *Simulación*): con el botón activo, clic en la tela en 3D
  sujeta ese punto donde quede al terminar de coser, por ejemplo para
  colgar un bolso de sus asas. Las fijaciones se guardan en el patrón y
  salen en el navegador junto a los broches; *Quitar* las borra todas.
- Clic en una pieza en la vista 3D la selecciona, también en el patrón, y
  se resalta en 3D.

**Ejemplos completos:** dos bolsos de verdad, de principio a fin, que
prueban todo junto.

- *Bolso con forro, bolsillo y cremallera* (`bolso-con-forro`): exterior de
  lona (delantero, trasero y una base que le da fondo), forro de algodón
  aparte con las mismas piezas, un bolsillo de parche cosido por tres lados
  a una U de líneas del forro, una cremallera con cada cinta cosida entre
  la boca del exterior y la del forro, y dos asas de cincha. El forro es
  tan ancho como la cremallera arriba y algo más estrecho justo debajo,
  como al coserlo con más margen, para que quepa dentro. Cosido no se
  cruza, el forro sigue dentro, las costuras cierran y, con la cremallera
  abierta, la boca se abre.
- *Chalk bag de escalada*, ya sin simplificar, con las piezas del patrón de
  Christian Halling: cuerpo de algodón con lona como entretela, base
  redonda, forro de pelo como bolsa aparte dentro del cuerpo y puño de
  punto doblado por la mitad, con sus dos bordes cosidos a la boca junto
  con el cuerpo y el forro, en una costura de cuatro lados. El puño y el
  forro se estiran de 341 a 381 mm sin encoger el cuerpo, nada se cruza y
  la bolsa se queda de pie al soltarla y derecha al colgarla, con mallas
  de 15 a 30 mm. Sus tres tubos (cuerpo, puño y forro) se cierran a la vez,
  luego la boca y por último la base: con el forro cerrado antes, tiraba
  de las bocas que esperaban abiertas, y con la base antes que la boca, el
  puño tiraba de lado del cuerpo ya cerrado por abajo y lo volcaba.
- Para que salgan bien, la colocación *alrededor* ahora:
  - pone cada forro a la altura de la costura que lo une a la capa de
    fuera (antes se alineaba con lo más alto, y el puño del chalk bag lo
    subía 13 cm por encima de la boca);
  - coloca los tubos (bolsas redondas) a su medida, sin el radio mínimo
    que los dejaba abiertos por detrás y los tumbaba al cerrarse, y varios
    tubos de la misma capa (el puño encima del cuerpo), cada uno en su
    cilindro, dando la misma vuelta: así encogen a la vez al cerrarse;
  - mete la base del forro dentro del exterior, para que este no la
    atrape debajo al cerrarse;
  - coloca la solapa de un doblez a la par de la tela de la que sale, en
    su propio cilindro, y la embebe lo que toca: alrededor de un tubo, la
    capa de dentro de un puño es algo más corta. Con las dos capas igual de
    largas, al cerrarse el tubo la de dentro no cabía, se arrugaba de golpe
    y volcaba la bolsa entera (el chalk bag colgado acababa boca abajo).
- El refuerzo a lo largo de una costura (la remallada, por ejemplo) ya no
  une puntos de un lado y otro de un doblez: a su largo en el patrón,
  desdoblaba el puño.
- Los cambios de forma en reposo de una pieza (fruncir o estirar junto a
  una costura, embeber una solapa) se suman en vez de pisarse.
- En el bolso con forro, el forro se cose antes que el exterior (su etapa
  va antes): si el exterior se cierra primero, sus costados se meten por
  los del forro, aún abiertos.

**Interfaz:** inspirada en los programas de CAD como Fusion 360, con tema
oscuro.

- Barra superior: menús, accesos rápidos (nuevo, abrir, guardar, deshacer,
  rehacer), el documento abierto (con un punto si hay cambios sin guardar) y
  la disposición de las vistas.
- Cinta de herramientas con las cinco etapas numeradas y *Siguiente* para
  pasar a la próxima. *Patrón*: seleccionar, crear (pluma, rectángulo,
  línea, doblez), modificar, inspeccionar (rejilla, malla) y encuadrar.
  *Telas*: tela de la selección (o de todas), añadir capa y tela nueva.
  *Montaje*: costura, piquete, broche, fornituras, tipo, margen, orden,
  voltear y resaltar. *Colocación*: lado, zona y cara de la
  pieza, su capa y sobre qué pieza va, maniquí, modo de colocación y coser. *Simulación*: simular / pausa,
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
`flat_felled`, `lapped`, `bagged` o `zipper`), `"pressed_to"` (índice del lado hacia el
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

Capas (opcional): cada pieza puede llevar `"layer"` (0 si falta: el
exterior; 1, el forro...) y `"on": { "piece": <id de pieza> }` para ir
encima de otra pieza, con `"wrong_side": true` si va por su revés.

Tramos, piquetes, ajuste y etapas (opcionales): un lado de costura puede
llevar `"end"` (id de la última arista si sigue el contorno por varias,
desde `edge` en el orden de la pieza) y `"range": [desde, hasta]` (fracciones
de su largo, en el orden de la pieza). Cada pieza puede llevar `"notches"`,
piquetes `{ "edge": 3, "t": 0.5 }` (`t` es el parámetro de la arista, de 0 a
1). Cada costura, `"fit"` (`exact`, `eased`, `gathered` o `stretched`; si
falta, `exact`) y `"step"` (id de su etapa). El patrón lleva sus etapas en
orden en `"steps"`: `[{ "id": 40, "name": "Costados" }, ...]`.

Dobleces, cremalleras y broches (opcionales): una línea interior con
`"fold": { "angle": 180, "pressed": true, "toward_right_side": false, "flip": false }`
es un doblez (ángulo en grados; `pressed`, si empieza doblado; hacia qué
cara se dobla; `flip` cambia qué lado es la solapa). Una costura de tipo
`zipper` puede llevar `"open"` (fracción abierta desde su principio, de 0 a
1). Cada pieza puede llevar `"marks"`, puntos marcados
`{ "id": 50, "at": { "x": 120, "y": 40 } }`, y el patrón, `"fasteners"`:
`[{ "id": 52, "name": "Broche 1", "kind": "snap", "ends": [{ "piece": 1, "mark": 50 }, { "piece": 10, "mark": 51 }], "open": false }]`
(`kind`: `snap`, `bar_tack` o `pin`; un broche abierto no une nada; una
fijación, `pin`, lleva sus dos extremos en el mismo punto y lo sujeta). Las
fornituras no tienen formato propio: son piezas y costuras normales.

## Desarrollo

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
