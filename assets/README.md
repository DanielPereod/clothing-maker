# Recursos

## `maniqui.glb`

Maniquí de la vista 3D. Es la malla base de [MakeHuman](http://www.makehumancommunity.org)
(`makehuman/data/3dobjs/base.obj`, malla «hm08»), que sus autores publicaron
con licencia **CC0** en septiembre de 2020:

> Copyright (C) 2020 Data Collection AB, Joel Palmius, Jonas Hauquier.
> Released as CC0.

CC0 permite usarla, modificarla y redistribuirla sin condiciones, también en
proyectos comerciales.

Se genera con [`tools/makehuman_a_glb.py`](../tools/makehuman_a_glb.py), que
se queda solo con el cuerpo (sin ojos, pestañas ni geometría de ayuda), lo
pasa a metros con los pies en el suelo y guarda las articulaciones como
nodos `joint-…` para calcular las zonas de colocación.
