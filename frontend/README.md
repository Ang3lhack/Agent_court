# Evidencia local y registro ICP · Etapa 6

El dashboard conserva el diseño oscuro y separa dos orígenes:

- **Evidencia local**: archivo JSON importado por el usuario, validado en el
  navegador. Permite elegir A/B, comparar hashes completos, ver la cadena y leer
  las pruebas, el código y el aislamiento exactamente como quedaron registrados.
- **Registro ICP**: consultas de solo lectura al canister existente. Conserva
  la paginación, los errores de conexión y la detención por desfase de reloj.
  Solo carga esta vista al seleccionarla. Los metadatos ICP no incluyen los
  informes completos ni el historial de eventos de los agentes.

No se sube el JSON, no se guarda en localStorage y no se publica automáticamente.
Cambiar de vista conserva el archivo importado; «Cerrar archivo» lo descarta de
la vista. Recargar la página requiere volver a importarlo. Una importación
fallida retira los datos anteriores para evitar mostrar resultados obsoletos.

## Demostración con el historial real existente

Desde la raíz del repositorio, con Node 24 y Rust disponibles:

```bash
COURT_VIEW_DIR="$(mktemp -d /tmp/agent-court-stage6.XXXXXX)"
node frontend/scripts/export-evidence.mjs \
  --evidence-dir .agent-court/demo-stage5d-real-0jJ6d5/evidence \
  --events .agent-court/demo-stage5d-real-0jJ6d5/orchestrate.log \
  --output "$COURT_VIEW_DIR/evidence.json"
printf 'Importa este archivo: %s\n' "$COURT_VIEW_DIR/evidence.json"
npm --prefix frontend run dev -- --mode evidence --host 127.0.0.1
```

Abre la URL que imprime Vite, selecciona **Evidencia local** e importa el JSON.
En WSL puedes acceder a `/tmp` desde `\\wsl.localhost\Ubuntu\tmp` (sustituye
`Ubuntu` por el nombre de tu distribución). Este modo usa el servidor Vite
existente y omite las consultas de configuración de ICP; no necesita iniciar
redes, desplegar canisters ni ejecutar Codex o Docker.

La exportación verifica los cuatro intentos con el comando Rust `verify`
existente. Conserva los bytes originales del historial y de cada archivo dentro
del JSON; no cambia predecesores, hashes, paquetes ni el log. La salida debe ser
un archivo **nuevo, fuera del almacén y de `.agent-court/`**. No pongas la
exportación en `public/`: incluye código, diagnósticos y el log local.

Al importar ese historial:

1. Aparecen los cuatro intentos en su orden original: PASS, PASS, FAIL, PASS.
2. A/B seleccionan inicialmente los dos últimos, el ciclo real FAIL → PASS.
3. Los hashes de artefacto e informe cambian; el hash de criterios coincide.
4. El informe FAIL conserva el error `successive increments`, `left: 2`,
   `right: 1`; el informe PASS conserva su resultado original.
5. El log solo vincula los dos últimos intentos. Builder figura como **omitido**
   en la demo controlada; Reviewer muestra su análisis y Repair su finalización
   según el log. El origen `REAL Codex CLI` se muestra como declaración del log,
   no como una atestación criptográfica. Los dos intentos anteriores quedan sin
   eventos asociados; el visor no inventa sus ejecuciones de agentes.

Para consultar también el backend **ya disponible**, usa el modo habitual:

```bash
ICP_ENVIRONMENT=local npm --prefix frontend run dev
```

Elige **Registro ICP**. No hace falta redesplegar. No se hace una asociación
automática entre los paquetes importados y publicaciones remotas, ni se afirma
que los registros locales estén certificados por ICP.

## Formato e integridad

`agent-court-local-v1` es un contenedor JSON independiente de las evidencias
originales; no cambia su esquema ni los contratos Candid:

```text
format: "agent-court-local-v1"
history: texto exacto de evidence.jsonl
packages: [{ attempt_id, manifest, report, payload, artifact, criteria }]
events: null | { text: texto exacto del log, sha256: hash de esa copia }
```

Los cinco archivos del paquete se conservan como cadenas UTF-8, incluyendo
saltos de línea. El exportador rechaza enlaces simbólicos en los archivos y
directorios de paquetes, UTF-8 inválido y destinos existentes. Invoca
`cargo run --quiet --locked --manifest-path evaluator/Cargo.toml --
--evidence-dir <almacén> verify <id>` para cada intento y después valida los
bytes capturados con el mismo validador que utiliza el navegador.

Ambos comprueban SHA-256 de criterios, artefacto, payload e informe completo;
correspondencia entre informe, payload, manifiesto e índice; IDs únicos;
veredictos y versiones admitidos; y cadena completa por misión. No basta con un
booleano `verified` proporcionado por el archivo. Un cambio de bytes, un paquete
ausente o un predecesor incompatible hacen que se rechace toda la importación.

Límites: 8 MiB por exportación, 100 intentos, 1 MiB por archivo o log y 2 MiB de
índice. Solo se admiten paquetes v1 e informes v2 completos con fechas enteras
seguras en JavaScript. Los registros antiguos de solo metadatos se rechazan,
sin reconstruirlos. SHA-256 en el navegador requiere HTTPS o localhost.

El análisis de eventos reconoce una única sesión `MISSION counter-v1` y enlaza
sus líneas `VERDICT` por ID, veredicto y predecesor con los paquetes verificados.
Si el log contradice esos paquetes, se rechaza. Un log sin sesión reconocible
se conserva como texto, sin atribuir ejecuciones; varias sesiones no se resumen
como una sola. Un evento START solo acredita que el log declara un inicio, no
que el agente terminó. No se infiere Repair a partir de un cambio FAIL → PASS.

**Límites de confianza:** coherencia SHA-256 local no autentica autoría,
ejecución, procedencia o fecha. Una persona puede reemplazar archivos y
recalcular hashes. El hash del log protege la copia frente a cambios accidentales
después de exportarla, pero ese log no pertenece a los paquetes ni tiene una
firma o certificación ICP. El visor muestra resultados conservados, no repite
las pruebas ni garantiza resistencia absoluta a código adversarial. El contador
de resultados refleja las entradas del informe; no inventa subpruebas.

## Validación

```bash
npm --prefix frontend test
npm --prefix frontend run build
npm --prefix frontend run lint
```

Las pruebas usan paquetes sintéticos explícitos en almacenes temporales.
Cubren integridad, cadena, errores, límites, log ausente/contradictorio,
procedencia real frente a simulada, renderizado de componentes, escape de texto
y exportación con el verificador Rust real. No ejecutan agentes, candidatos,
Docker ni publicaciones. El renderizado se prueba con React y TypeScript ya
instalados; no se añadieron dependencias ni un framework de navegador.

Comprobación manual de interfaz: importar el JSON real, cambiar A/B, desplegar
código y aislamiento, seleccionar un intento anterior sin eventos, cerrar y
volver a importar. Probar también un JSON alterado y confirmar el rechazo sin
datos anteriores. Revisar teclado y ancho móvil; las tablas pueden desplazarse
horizontalmente y los informes se apilan en pantallas pequeñas.
