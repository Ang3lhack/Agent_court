# Evaluador Agent Court: evidencias locales

El evaluador compila y ejecuta las tres pruebas reales del fixture confiable
`counter-v1`. Conserva los bytes originales de criterios, fixture e informe
para poder verificar su integridad después. Solo usa Rust estándar, serde,
serde_json y sha2; requiere Rust 1.89 o posterior por el bloqueo de archivos.

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
bloqueo al cerrar el proceso. El archivo `.writer.lock` puede permanecer vacío;
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

No se almacenan credenciales, no se invoca Codex al evaluar el fixture o verificar,
y no hay publicación hacia ICP en esta etapa.

## Pruebas

```sh
cargo test --manifest-path evaluator/Cargo.toml
cargo fmt --manifest-path evaluator/Cargo.toml -- --check
cargo check --manifest-path evaluator/Cargo.toml --locked
cargo clippy --manifest-path evaluator/Cargo.toml --all-targets -- -D warnings
```

Las pruebas de persistencia y alteración crean sus propios directorios temporales;
no borran ni modifican el historial real del repositorio.
