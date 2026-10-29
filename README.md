# Agent Court

Agent Court es un MVP para evaluar una misión pequeña con Rust y conservar metadatos de sus intentos en un registro local y en un canister de Internet Computer (ICP).

## Qué contiene

| Carpeta | Función |
| --- | --- |
| `evaluator/` | Ejecutor Rust de `counter-v1` con paquetes SHA-256, almacén configurable, `verify` y publicación explícita de metadatos verificados mediante `publish` hacia ICP local. También ofrece un adaptador opcional para Codex CLI. |
| `backend/` | Canister Rust que guarda misiones e intentos en memoria estable y expone consultas Candid. Las actualizaciones requieren que quien llama sea controlador del canister. |
| `frontend/` | Dashboard React/Vite que genera bindings Candid y consulta las misiones e intentos reales del backend. |
| `icp.yaml` | Declara los canisters que construye y despliega `icp-cli`. |

El evaluador actual ejecuta únicamente un fixture Rust confiable incluido en el código; todavía no ejecuta código generado por agentes ni constituye una sandbox del sistema operativo. El log JSONL local es append-only desde el programa, pero no es resistente a manipulación. Los hashes identifican bytes y no demuestran que el código sea correcto. `publish` verifica un paquete y registra sus ocho campos originales explícitamente en ICP local, con control de duplicados y cadena remota.

## Requisitos para Windows

Para desarrollar y levantar la red local, usa **WSL 2 con Ubuntu y Docker Desktop**. Ejecuta los comandos de compilación dentro de Ubuntu/WSL, no en PowerShell ni en Git Bash. Puedes editar el proyecto con Visual Studio Code y su extensión **WSL**.

### 1. Instalar WSL 2 y Ubuntu

Abre PowerShell como administrador y ejecuta:

```powershell
wsl --install -d Ubuntu-24.04
```

Reinicia Windows si lo solicita. Abre Ubuntu desde el menú Inicio, crea el usuario Linux y actualiza los paquetes:

```bash
sudo apt update
sudo apt upgrade -y
sudo apt install -y build-essential curl git pkg-config libssl-dev ca-certificates
```

### 2. Instalar Docker Desktop

Instala [Docker Desktop para Windows](https://docs.docker.com/desktop/setup/install/windows-install/). En sus ajustes:

1. Activa **Use the WSL 2 based engine**.
2. En **Resources → WSL Integration**, activa la integración para Ubuntu-24.04.
3. Inicia Docker Desktop y confirma desde Ubuntu:

```bash
docker --version
docker info
```

La red de desarrollo local de `icp-cli` necesita Docker Desktop en Windows.

### 3. Instalar Node.js 24 en WSL

El frontend declara Node 24 en `frontend/.node-version`. Instálalo dentro de Ubuntu con nvm:

```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
source ~/.bashrc
nvm install 24
nvm use 24
nvm alias default 24
node --version
npm --version
```

### 4. Instalar Rust y el target WebAssembly

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup update stable
rustup target add wasm32-unknown-unknown
rustc --version
cargo --version
```

### 5. Instalar las herramientas de ICP

```bash
npm install -g @icp-sdk/icp-cli @icp-sdk/ic-wasm
icp --version
ic-wasm --version
```

### 6. (Opcional) Instalar Codex CLI

Solo hace falta para usar el comando `agent` del evaluador. Instálalo y autentícate con la cuenta que quieras usar:

```bash
npm install -g @openai/codex@latest
codex --version
codex login
codex login status
```

El adaptador usa `codex exec`; el rol `reviewer` pide sandbox `read-only` y `builder`/`repair` piden `workspace-write`. La disponibilidad del comando no sustituye una sandbox de seguridad para código no confiable.

## Clonar y preparar el proyecto

En la terminal Ubuntu de WSL:

```bash
git clone https://github.com/Ang3lhack/Agent_court.git
cd Agent_court
nvm use 24
npm --prefix frontend install
cargo fetch --manifest-path backend/Cargo.toml
cargo fetch --manifest-path evaluator/Cargo.toml
```

Si abres una terminal nueva, vuelve a entrar a la carpeta del repositorio y ejecuta `nvm use 24`.

Para editar desde Visual Studio Code, instala la extensión **WSL** y, desde la carpeta del proyecto en Ubuntu, ejecuta:

```bash
code .
```

La ventana de VS Code debe indicar que está conectada a WSL. Usa su terminal integrada para los siguientes comandos.

## Compilar y ejecutar el evaluador local

Desde la raíz del repositorio:

```bash
cargo test --manifest-path evaluator/Cargo.toml
cargo run --manifest-path evaluator/Cargo.toml
```

El segundo comando compila y corre tres pruebas Rust del fixture `counter-v1`, imprime un informe JSON y añade un intento a `.agent-court/evidence.jsonl`. Ese directorio está ignorado por Git.

Para la demostración de la etapa 4, conserva ese historial y usa un almacén
independiente con `--evidence-dir .agent-court/demo-stage4`. Consulta los
[comandos de generación, verificación y primera publicación manual](evaluator/README.md#primera-publicación-manual-de-counter-v1).

Para ejecutar Codex de forma opcional, reemplaza la ruta y el prompt por los valores que necesites:

```bash
cargo run --manifest-path evaluator/Cargo.toml -- agent reviewer ./backend "Revisa el código y reporta hallazgos sin modificar archivos."
```

Roles admitidos: `builder`, `reviewer` y `repair`. `builder` y `repair` pueden modificar el directorio indicado; revisa el prompt y los cambios resultantes.

## Compilar, probar y desplegar los canisters localmente

Pruebas del canister Rust y compilación del proyecto ICP:

```bash
cargo test --manifest-path backend/Cargo.toml
icp build
```

Inicia la red local y despliega los canisters declarados en `icp.yaml`:

```bash
icp network start -d
icp deploy
```

Consulta las misiones registradas (al inicio puede devolver una lista vacía):

```bash
icp canister call backend list_missions '(null, 20)'
```

Al terminar:

```bash
icp network stop
```

El backend restringe `register_mission` y `register_attempt` a controladores del canister; los métodos de consulta son públicos. El deploy local sirve para desarrollo. Antes de publicar en una red pública, revisa identidades, controladores y el modelo de seguridad del canister.

## Desarrollo del frontend

La etapa 6 permite comparar paquetes locales verificados y consultar por
separado los metadatos ICP. Para explorar el historial real de Reviewer/Repair
sin iniciar redes ni desplegar, sigue la [demostración local del frontend](frontend/README.md).
El comando `npm --prefix frontend run dev -- --mode evidence` abre ese modo con
el Vite existente. Los archivos importados no se suben ni se presentan como
certificados por ICP.

El servidor Vite consulta el estado de la red local y el ID del canister backend, por lo que primero debes iniciar la red y desplegar el backend. En una segunda terminal WSL:

```bash
icp network start -d
icp deploy backend
npm --prefix frontend run dev
```

Vite imprime la dirección local, normalmente `http://localhost:5173`. Para detener Vite pulsa `Ctrl+C`; para parar la red, ejecuta `icp network stop`.

**Estado actual:** el frontend consulta las misiones e intentos del backend real.
Si la red y el canister ya están disponibles, basta con ejecutar
`ICP_ENVIRONMENT=local npm --prefix frontend run dev`; no los redespliegues para
visualizar una publicación de la etapa 4.

## Comandos de referencia

```bash
# Compilar el backend WebAssembly
icp build backend

# Construir el frontend
npm --prefix frontend run build

# Revisar estilo del frontend
npm --prefix frontend run lint

# Ejecutar pruebas del backend y del evaluador
cargo test --manifest-path backend/Cargo.toml
cargo test --manifest-path evaluator/Cargo.toml
```

## Publicar cambios en GitHub

La primera vez, confirma que estás en el repositorio correcto y que GitHub te autenticó. Con Git for Windows puedes configurar Git Credential Manager; también puedes instalar GitHub CLI y ejecutar `gh auth login`. No pegues tokens en los comandos ni los guardes en este repositorio.

```bash
git remote -v
git status
git add README.md
git commit -m "docs: add Windows and WSL setup guide"
git push -u origin main
```

Si `origin` aún no está configurado, agrégalo antes del push:

```bash
git remote add origin https://github.com/Ang3lhack/Agent_court.git
```

Si el remoto ya tiene commits que no existen localmente, integra primero la rama remota después de revisar sus cambios; no uses `git push --force` para resolverlo.

## Documentación específica

- [Registro ICP backend](backend/README.md)
- [Evaluador Rust](evaluator/README.md)
- [Documentación de icp-cli](https://cli.internetcomputer.org/)
- [Inicio rápido para ICP](https://docs.internetcomputer.org/getting-started/quickstart/)
