# Evaluador Agent Court: evidencias locales y publicación ICP

El evaluador compila y ejecuta las tres pruebas reales del fixture confiable
`counter-v1`. Conserva los bytes originales de criterios, fixture e informe
para poder verificar su integridad después. Usa Rust estándar, serde,
serde_json, sha2, candid y hex; requiere Rust 1.89 o posterior por el bloqueo
de archivos. La publicación es explícita y reutiliza `icp-cli` 1.7.0.

## Generar y verificar

Ejecuta desde la raíz del repositorio; `.agent-court/` se resuelve respecto al
directorio de trabajo, no respecto a Cargo.toml.

```sh
cargo run --manifest-path evaluator/Cargo.toml
cargo run --manifest-path evaluator/Cargo.toml -- verify ATTEMPT_ID
```

Sustituye `ATTEMPT_ID` por el campo `evidence.attempt_id` que imprime la primera
ejecución. Las pruebas también imprimen su salida en consola; el paquete
archivado y el JSONL son las fuentes persistentes para otras herramientas.

La opción `--evidence-dir DIRECTORIO` debe ir **antes** de `verify` o `publish`;
sin subcomando selecciona el almacén donde se genera el fixture:

```sh
cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4
cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4 verify ATTEMPT_ID
cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4 publish ATTEMPT_ID --identity agent-court-local
```

El valor predeterminado sigue siendo `.agent-court/`. Cada almacén tiene su
propio índice, paquetes y bloqueo; el primer intento de cada misión lleva
`previous_attempt_id = null`. La ruta se resuelve desde el directorio de trabajo.
La opción no se aplica a `agent`, cuyos argumentos y prompts se conservan.

`verify` no compila ni ejecuta el artefacto, no llama a ICP, no crea archivos ni
reescribe evidencias. Lee el índice y el paquete solicitado, recalcula los
digests y comprueba la coherencia de metadatos y payload.

Códigos de salida de `verify`:

| Código | Significado |
| --- | --- |
| 0 | Integridad del paquete comprobada. |
| 2 | Archivo ausente/modificado, metadatos incoherentes, intento no registrado, formato desconocido o error de lectura. |
| 3 | Registro histórico de solo metadatos: no verificable. |
| 64 | Uso incorrecto de la CLI. |

El modo sin argumentos conserva los veredictos PASS, FAIL, ERROR y NOT_RUN.
Un fallo de persistencia devuelve 2 y no anuncia una evaluación completa.
El veredicto de evaluación y el resultado de verificar integridad son conceptos
diferentes: verificar correctamente un paquete FAIL no convierte el código en PASS.

El subcomando existente sigue disponible:

```sh
cargo run --manifest-path evaluator/Cargo.toml -- agent reviewer ./backend "Revisa sin modificar archivos."
```

Ese comando **sí invoca Codex**. No forma parte de la generación/verificación
de este paquete ni se usa en sus pruebas. No se ha integrado código generado
por agentes con el evaluador. Los argumentos desconocidos devuelven 64 para
evitar generar un intento accidentalmente.

## Archivos del paquete

```text
.agent-court/
  .writer.lock
  evidence.jsonl
  attempts/
    ATTEMPT_ID/
      criteria.txt
      artifact.rs
      report-payload.json
      report.json
      manifest.json
```

| Archivo | Contenido y comprobación |
| --- | --- |
| `criteria.txt` | Bytes UTF-8 exactos de los criterios originales, sin agregar salto de línea; SHA-256 = `criteria_hash`. |
| `artifact.rs` | Bytes exactos del fixture Rust evaluado, incluidas sus tres pruebas; SHA-256 = `artifact_hash`. |
| `report-payload.json` | Bytes JSON exactos empleados para el digest del informe; SHA-256 = `report_hash`. |
| `report.json` | Informe estructurado completo, incluido `report_hash`; su checksum adicional es `manifest.report_file_hash`. |
| `manifest.json` | `record` contiene misión, intento, predecesor, tres hashes, veredicto, timestamp en ms y `evidence_format_version: 1`. También contiene `report_file_hash`. |

El registro JSONL de un intento nuevo incluye
`evidence_format_version: 1`. Los registros históricos no tienen ese campo;
permanecen intactos y no se les asignan archivos reconstruidos.

Los criterios y el fixture no cambian. Sus digests de compatibilidad son:

- Criterios: `7436340ba6690040ea50fdcf786ed1c23b9595f326d895002f560538eb23d28f`.
- Fixture: `5ad72d8d4c8bae96ccd0979229a3d98f6d3d582fe85625da83a706a51efc9d57`.

## Definición del hash del informe

Los informes nuevos usan `schema_version: 2`. Su payload contiene todos los
campos del informe excepto su propio `report_hash`:

- `schema_version`
- `mission_id`
- `verdict`
- `checks`
- `criteria_hash`
- `artifact_hash`
- `timestamp_unix_ms`
- `isolation`

El payload se serializa una sola vez durante la evaluación. Su vector de bytes
se usa tanto para SHA-256 como para escribir `report-payload.json`, sin añadir
saltos de línea ni volver a serializarlo para verificar su digest. No se exige
una canonicalización JSON universal: se verifican esos bytes archivados.

`report.json` incluye el digest del payload y tiene un checksum independiente
en el manifiesto. Esto evita la autorreferencia y también permite detectar
cambios de espacios o formato en el informe completo. El verificador compara
el contenido del payload con los campos del informe, además de recalcular los
hashes. El timestamp del manifiesto, del JSONL y del informe nuevo es el mismo.

El esquema anterior calculaba el digest sobre un payload no archivado y no
incluía `isolation`. No se reinterpretan ni se recalculan esos registros antiguos.

## Escritura y compatibilidad

Solo se permite **un escritor por almacén**. El bloqueo exclusivo no bloqueante
de `std::fs::File` se mantiene durante la persistencia: otro escritor recibe
un error y debe reintentar secuencialmente. El sistema operativo libera el
bloqueo al cerrar el proceso. Al terminar la persistencia se desbloquea también
explícitamente, evitando retenerlo si un proceso hijo concurrente heredó
brevemente el descriptor antes de exec. El archivo `.writer.lock` puede permanecer vacío;
no indica por sí mismo que haya una evaluación activa y no debe borrarse
mientras se usa el almacén. No hay una cola ni un coordinador distribuido.

Orden de escritura:

1. Leer y validar el índice existente, sin normalizar sus bytes.
2. Crear un directorio de intento exclusivo y archivos con `create_new`.
3. Escribir y sincronizar los cuatro archivos de contenido.
4. Escribir el manifiesto y sincronizar el paquete y su directorio padre.
5. Escribir y sincronizar un índice temporal que contiene el prefijo histórico
   exacto más la nueva línea JSONL.
6. Sustituir atómicamente el índice mediante rename dentro del mismo directorio
   y sincronizar ese directorio en Unix/WSL.

El índice conserva todos los bytes anteriores como prefijo, incluidos espacios
y terminaciones de línea. Si el último registro válido no tenía un salto final,
se añade un separador antes del nuevo registro. No se reserializa el historial.
Se conservan los permisos del índice existente.

Un directorio parcial o completo que no aparezca en `evidence.jsonl` **no se
considera registrado**. Si algo falla, se devuelve error; los restos nuevos se
conservan para inspección y no se eliminan paquetes antiguos. No hay reparación
ni recuperación automática de restos o índices manipulados. Un JSONL truncado,
inválido o con IDs duplicados se rechaza sin reescribirlo.

Las garantías de rename y sincronización dependen del sistema de archivos. Las
pruebas se realizan con el entorno Linux/WSL del proyecto; en plataformas no
Unix no se sincronizan directorios. Si falla la sincronización después del
rename, la CLI informa error y la durabilidad del commit queda incierta, aunque
el índice completo ya pueda ser visible. Nunca se promete atomicidad conjunta
de varios archivos frente a cualquier fallo de hardware.

Los bloqueos son cooperativos: no ejecutes simultáneamente binarios antiguos
que no los respetan ni edites el almacén durante la escritura.

## Alcance de la verificación

Un resultado VERIFICADO confirma que los bytes leídos coinciden con los digests
del índice/manifiesto y que los metadatos, el informe y su payload son coherentes.
Comprueba también que el predecesor corresponde al intento anterior de la misma
misión en el índice. No verifica recursivamente los paquetes de todos los
predecesores ni inventa artefactos para los registros históricos.

**SHA-256 no demuestra que el código sea correcto**, que una prueba se ejecutó
honestamente, ni que el veredicto o el reloj sean fiables. Este almacén local no
tiene firmas ni un anclaje externo de confianza: quien pueda modificar todos
los archivos y digests coherentemente puede falsificarlo. Tampoco hay prueba de
que no se hayan eliminado entradas completas del historial.

Los IDs se validan como slugs y los archivos del paquete no pueden ser enlaces
simbólicos. Esto no es una sandbox ni una defensa completa frente a un sistema
de archivos hostil; el directorio local y sus padres deben ser de confianza.

No se almacenan credenciales ni se invoca Codex al evaluar, verificar o publicar.

## Publicar un intento verificado (etapa 4)

```sh
cargo run --manifest-path evaluator/Cargo.toml -- publish ATTEMPT_ID
```

Este comando usa el almacén predeterminado, la identidad `agent-court-local`
y el canister `backend` del entorno `local`. Solo se admite una red administrada
en loopback. No hay opciones para mainnet, despliegue o cambio de controladores.
Las variables heredadas `ICP_NETWORK`, `ICP_ENVIRONMENT` e `ICP_PROJECT_ROOT`
no pueden cambiar el destino del proceso hijo. El proyecto ICP se resuelve
respecto al checkout donde se compiló el evaluador.

El flujo es:

1. Ejecutar el mismo verificador local de paquetes de `verify`. Los registros
   históricos sin archivos y los paquetes modificados se rechazan antes de llamar a ICP.
2. Validar IDs y hashes y convertir el timestamp `u128` a `nat64` con `u64::try_from`.
   Un timestamp mayor que `u64::MAX` se rechaza, sin truncar ni ajustar el reloj archivado.
3. Consultar `get_mission` y exigir el mismo `mission_id` y `criteria_hash`.
4. Consultar `get_attempt`. Si ya existe y coinciden **los ocho campos originales**,
   terminar con `YA REGISTRADO`, sin escritura. Si difiere un campo, informar conflicto.
5. Leer todas las páginas de `list_attempts` (100 entradas por página) y reconstruir
   la punta por enlaces `previous_attempt_id`. La lista se ordena por timestamp del
   cliente, que puede retroceder; no se toma su última fila como punta.
   Se rechazan cadenas incompletas, bifurcadas, repetidas o desconectadas.
6. Exigir que el predecesor local sea la punta remota y llamar una sola vez a
   `register_attempt`, conservando exactamente misión, ID, predecesor, tres hashes,
   veredicto y timestamp. Se envían solo los ocho campos de `AttemptInput`;
   la versión del paquete local no se envía. Las variantes son `PASS`, `FAIL`,
   `ERROR` y `NOT_RUN`. Se puede publicar la integridad verificada de un informe FAIL.
7. Comprobar el `Ok` tipado y sus metadatos, volver a consultar `get_attempt` y
   comparar tanto los campos originales como la respuesta de registro completa.
   Solo entonces informar `REGISTRADO`.

El transporte usa `std::process::Command` con argumentos separados, sin shell.
`candid` codifica los argumentos binarios, `--args-format hex` los entrega a ICP
y `--output hex` permite decodificar la respuesta binaria mediante tipos Rust
correspondientes a `backend/backend.did`. Se comprobó también la salida textual
real: `(variant { Ok = opt record { ... } })`, `(variant { Ok = null })` y
`(variant { Ok = vec {} })`. No se analiza Candid mediante búsquedas de texto.

### Errores, carreras e idempotencia

`publish` devuelve 0 únicamente después de confirmar el contenido remoto y 2
ante fallo o conflicto; el uso incorrecto devuelve 64. Un proceso `icp` con salida
0 que devuelve `variant { Err = ... }` sigue siendo un error de aplicación.

Se distinguen `Unauthorized`, `NotFound`, `AlreadyExists`, `ChainConflict`,
`InvalidInput`, `CapacityReached` y `StorageError`, además de respuestas Candid
inválidas o desconocidas y errores del proceso/conexión/identidad/expiry.
`Unauthorized` requiere utilizar una identidad que ya sea controladora;
el publicador no cambia los controladores.

Una carrera que devuelve `AlreadyExists` obliga a releer y comparar los ocho campos.
`ChainConflict` siempre termina con error: muestra el predecesor esperado y nunca
reescribe el intento ni fabrica un éxito. Si cambia la punta entre la consulta y
el registro, el backend sigue siendo la autoridad que rechaza el registro.

Si la respuesta del update se pierde o no se reconoce, se consulta `get_attempt`
antes de decidir el resultado. Un registro idéntico visible permite informar
`REGISTRO CONFIRMADO tras respuesta incierta`; si no aparece o tampoco puede leerse,
se devuelve error de estado sin confirmar. No se reenvía el update automáticamente.
Al repetir **el mismo comando y el mismo ID**, se vuelve a verificar y reconciliar
antes de cualquier escritura. Ningún resultado de publicación modifica el almacén.

### Identidad y contraseña

Las consultas públicas se hacen con `--identity anonymous`; solo el update usa
`--identity agent-court-local` o el nombre indicado por el usuario. En una terminal
interactiva se heredan stdin y stderr para conservar el prompt oficial de
`icp-cli` (dialoguer exige que stderr sea una terminal). ICP muestra directamente
sus diagnósticos; el evaluador mantiene el fallo y consulta el ID para reconciliar.
En modo sin terminal se capturan los errores y se clasifican explícitamente
conexión, identidad/contraseña no disponible e `Invalid request expiry`.

Si la identidad necesita desbloqueo, ejecuta la publicación manualmente desde
una terminal interactiva. El evaluador no lee la contraseña ni la pasa por
argumentos, archivos o variables de entorno. No almacena ni exporta claves,
tokens, sesiones o credenciales. La gestión de identidad permanece en ICP CLI.
Ante `Invalid request expiry`, comprueba la sincronización del reloj del host/WSL
y de la réplica; nunca cambies el timestamp archivado para solucionarlo.

## Primera publicación manual de counter-v1

La misión ya está registrada en la red local existente. No despliegues ni
reinstales canisters para este procedimiento. En la validación de esta etapa se
leyó su hash de criterios correcto y cero intentos mediante ICP CLI y el adaptador
Rust, **sin ejecutar un update real**. La primera escritura queda para el usuario.

Los cuatro registros originales de `.agent-court/evidence.jsonl` se conservan.
Los tres primeros no tienen paquetes y no son publicables. El cuarto es verificable
pero depende del tercero: con ICP vacío se rechaza con `ChainConflict`.
El almacén nuevo resuelve esa diferencia sin modificar el historial.

Desde la raíz del repositorio, en Bash/WSL:

```sh
# 1. Generar SOLO si este almacén todavía no existe.
test ! -e .agent-court/demo-stage4 && \
  cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4

# Copia evidence.attempt_id de la salida y pégalo cuando read espere una línea.
read -r -p 'attempt_id generado: ' ATTEMPT_ID

# 2. Verificar los archivos y sus hashes.
cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4 verify "$ATTEMPT_ID"

# 3. Publicar manualmente; responde solo al prompt oficial de ICP si aparece.
cargo run --manifest-path evaluator/Cargo.toml -- --evidence-dir .agent-court/demo-stage4 publish "$ATTEMPT_ID" --identity agent-court-local

# 4. Leer el historial y el intento concreto.
icp canister call backend list_attempts '("counter-v1", null, 100 : nat16)' --environment local --identity anonymous --query --output candid --candid backend/backend.did
icp canister call backend get_attempt "(\"$ATTEMPT_ID\")" --environment local --identity anonymous --query --output candid --candid backend/backend.did

# 5. Abrir Vite usando la red y el backend ya existentes.
ICP_ENVIRONMENT=local npm --prefix frontend run dev
```

Abre la URL que imprima Vite (habitualmente `http://localhost:5173`), actualiza
las misiones, selecciona `counter-v1` y actualiza sus intentos. El dashboard
mostrará los metadatos publicados. No necesita un deploy nuevo. Si el directorio
demo ya existe, conserva sus archivos: reutiliza su primer ID o elige otro
directorio vacío, teniendo en cuenta la punta remota actual.

No se ha validado una publicación real exitosa en esta etapa. Sí se validaron
el flujo completo con mocks/procesos controlados y las consultas reales de lectura.

## Pruebas

```sh
cargo test --manifest-path evaluator/Cargo.toml
cargo fmt --manifest-path evaluator/Cargo.toml -- --check
cargo check --manifest-path evaluator/Cargo.toml --locked
cargo clippy --manifest-path evaluator/Cargo.toml --all-targets -- -D warnings
```

Las pruebas de persistencia y alteración crean sus propios directorios temporales;
no borran ni modifican el historial real del repositorio.

Cubren metadatos Candid y los cuatro veredictos, paquetes alterados/legacy,
misión inexistente, criterios distintos, duplicados idénticos/diferentes,
predecesores incorrectos, paginación y relojes regresivos, cada `RegistryError`,
errores de red antes y después de confirmar, respuestas inválidas y límites
`nat64`. Las pruebas CLI ejecutan el fixture confiable en un almacén temporal,
verifican sus hashes y prueban `agent` con un ejecutable Codex **falso** para los
tres roles. No ejecutan agentes reales ni escriben en ICP.

La única prueba de integración con ICP está ignorada de forma predeterminada
y hace exclusivamente consultas. Para usar la red local ya existente:

```sh
cargo test --manifest-path evaluator/Cargo.toml --locked read_only_local_icp -- --ignored --nocapture
```

Se consultaron la [skill ICP CLI](https://skills.internetcomputer.org/.well-known/skills/icp-cli/SKILL.md),
la [guía de agentes ICP](https://docs.internetcomputer.org/guides/ai-coding-agents/)
y los [ejemplos oficiales de DFINITY](https://github.com/dfinity/examples).
El comportamiento del proceso se contrastó con `icp --version`, `icp canister call --help`
y el [código oficial de ICP CLI 1.7.0](https://github.com/dfinity/icp-cli/blob/v1.7.0/crates/icp-cli/src/commands/canister/call.rs).

Queda pendiente integrar artefactos generados por Codex, definir su ejecución
aislada y construir el flujo Builder → Reviewer → Repair. Esta etapa solo publica
metadatos de paquetes ya verificados del fixture confiable.
