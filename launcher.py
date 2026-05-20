import json
import shlex
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import List

try:
    from textual.app import App, ComposeResult
    from textual.containers import Horizontal, Vertical
    from textual.reactive import reactive
    from textual.widgets import Button, Footer, Header, Input, Label, ListItem, ListView, Log, Select, Static, Switch
except ImportError as exc:
    raise SystemExit(
        "La dependance 'textual' est manquante. Installe-la avec: pip install textual"
    ) from exc


APP_DIR = Path(__file__).resolve().parent
STATE_DIR = APP_DIR / ".launcher"
GLOBAL_FILE = STATE_DIR / "global.json"
PROFILES_FILE = STATE_DIR / "profiles.json"
PID_FILE = STATE_DIR / "llama-server.pid"
LOG_OUT = STATE_DIR / "llama-server.log"
LOG_ERR = STATE_DIR / "llama-server.err.log"


@dataclass
class GlobalSettings:
    llama_server_path: str = ""
    model_dirs: List[str] = None

    def __post_init__(self) -> None:
        if self.model_dirs is None:
            self.model_dirs = []


@dataclass
class Profile:
    name: str = "default"
    model_path: str = ""
    host: str = "127.0.0.1"
    port: int = 8080
    ctx_size: int = 4096
    threads: int = 8
    n_gpu_layers: int = 0
    temp: float = 0.7
    top_p: float = 0.95
    batch_size: int = 512
    embeddings: bool = False
    flash_attn_mode: str = "off"  # off|on|auto
    extra_args: str = ""


def ensure_state() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)


def load_global() -> GlobalSettings:
    ensure_state()
    if not GLOBAL_FILE.exists():
        return GlobalSettings()
    try:
        data = json.loads(GLOBAL_FILE.read_text(encoding="utf-8"))
        return GlobalSettings(
            llama_server_path=data.get("llama_server_path", ""),
            model_dirs=data.get("model_dirs", []),
        )
    except Exception:
        return GlobalSettings()


def save_global(settings: GlobalSettings) -> None:
    ensure_state()
    GLOBAL_FILE.write_text(json.dumps(asdict(settings), indent=2), encoding="utf-8")


def load_profiles() -> List[Profile]:
    ensure_state()
    if not PROFILES_FILE.exists():
        return [Profile()]
    try:
        raw = json.loads(PROFILES_FILE.read_text(encoding="utf-8"))
        entries = raw.get("profiles", [])
        profiles: List[Profile] = []
        for item in entries:
            if not isinstance(item, dict):
                continue
            if "flash_attn" in item and "flash_attn_mode" not in item:
                item["flash_attn_mode"] = "on" if item.get("flash_attn") else "off"
            item.pop("flash_attn", None)
            profiles.append(Profile(**item))
        return profiles or [Profile()]
    except Exception:
        return [Profile()]


def save_profiles(profiles: List[Profile]) -> None:
    ensure_state()
    payload = {"profiles": [asdict(p) for p in profiles]}
    PROFILES_FILE.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def resolve_llama_server_path(raw: str) -> Path:
    p = Path(raw.strip().strip('"'))
    if p.is_dir():
        p = p / "llama-server.exe"
    return p


def pid_value() -> int:
    if not PID_FILE.exists():
        return 0
    try:
        return int(PID_FILE.read_text(encoding="utf-8").strip())
    except Exception:
        return 0


def is_running() -> bool:
    pid = pid_value()
    if pid <= 0:
        return False
    try:
        subprocess.run(
            ["tasklist", "/FI", f"PID eq {pid}"],
            check=False,
            capture_output=True,
            text=True,
        )
        return True
    except Exception:
        return False


class LlamaLauncherApp(App):
    CSS = """
    #root { layout: horizontal; height: 1fr; }
    #left { width: 30; padding: 0 1; border: round #666666; }
    #center { width: 1fr; padding: 0 1; border: round #666666; }
    #right { width: 38; padding: 0 1; border: round #666666; }
    .row { height: auto; margin: 0; }
    .title { text-style: bold; margin: 0 0 1 0; }
    .pair { height: 3; }
    .pair Label { width: 12; }
    #status { height: 3; border: round #555555; padding: 0 1; margin-top: 1; }
    #profile_grid { height: auto; }
    #profile_col_left, #profile_col_right { width: 1fr; }
    Log { height: 1fr; border: round #444444; }
    """

    active_profile_index = reactive(0)
    logs_following = reactive(False)
    active_log_file = reactive("stdout")

    def __init__(self) -> None:
        super().__init__()
        self.global_settings = load_global()
        self.profiles = load_profiles()
        self._last_log_size = 0

    @property
    def profile(self) -> Profile:
        return self.profiles[self.active_profile_index]

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with Horizontal(id="root"):
            with Vertical(id="left"):
                yield Label("Profils", classes="title")
                yield ListView(id="profiles")
                yield Button("Nouveau profil", id="profile_new")
                yield Button("Supprimer profil", id="profile_delete")
                yield Static("", id="status")
            with Vertical(id="center"):
                yield Label("Reglages profil", classes="title")
                with Horizontal(id="profile_grid"):
                    with Vertical(id="profile_col_left"):
                        yield Horizontal(Label("Nom"), Input(placeholder="profil", id="p_name"), classes="pair")
                        yield Horizontal(Label("Modele"), Input(placeholder=".gguf", id="p_model"), classes="pair")
                        yield Select([], prompt="Modele detecte", id="p_model_select")
                        yield Horizontal(Label("Host"), Input(value="127.0.0.1", id="p_host"), classes="pair")
                        yield Horizontal(Label("Port"), Input(value="8080", id="p_port"), classes="pair")
                    with Vertical(id="profile_col_right"):
                        yield Horizontal(Label("Ctx"), Input(value="4096", id="p_ctx"), classes="pair")
                        yield Horizontal(Label("Threads"), Input(value="8", id="p_threads"), classes="pair")
                        yield Horizontal(Label("GPU"), Input(value="0", id="p_gpu"), classes="pair")
                        yield Horizontal(Label("Temp"), Input(value="0.7", id="p_temp"), classes="pair")
                        yield Horizontal(Label("Top-p"), Input(value="0.95", id="p_top_p"), classes="pair")
                        yield Horizontal(Label("Batch"), Input(value="512", id="p_batch"), classes="pair")
                yield Horizontal(
                    Label("Embeddings"),
                    Switch(id="p_embeddings"),
                    Label("Flash-attn"),
                    Select([("off", "off"), ("on", "on"), ("auto", "auto")], id="p_flash"),
                    classes="row",
                )
                yield Input(placeholder="Extra args", id="p_extra")
                yield Horizontal(
                    Button("Sauver profil", id="save_profile"),
                    Button("Launch", id="launch"),
                    Button("Stop", id="stop"),
                    Button("Restart", id="restart"),
                    classes="row",
                )
            with Vertical(id="right"):
                yield Label("Global + Logs", classes="title")
                yield Horizontal(Label("Server"), Input(placeholder="llama-server.exe", id="g_server"), classes="pair")
                yield Horizontal(Label("Add dir"), Input(placeholder="dossier modeles", id="g_add_dir"), Button("+", id="g_add_dir_btn"), classes="pair")
                yield Select([], prompt="Supprimer dossier modeles", id="g_del_dir")
                yield Button("Supprimer dossier", id="g_del_dir_btn")
                yield Button("Sauver global", id="save_global")
                yield Horizontal(
                    Button("Voir stdout", id="log_stdout"),
                    Button("Voir stderr", id="log_stderr"),
                    Button("Live tail on/off", id="log_tail"),
                    classes="row",
                )
                yield Log(id="logs")
        yield Footer()

    def on_mount(self) -> None:
        self.refresh_profiles_list()
        self.load_profile_to_ui()
        self.load_global_to_ui()
        self.refresh_model_select()
        self.refresh_model_dirs_select()
        self.set_status("Pret.")
        self.set_interval(0.6, self.tick_tail)

    def set_status(self, text: str) -> None:
        self.query_one("#status", Static).update(text)

    def refresh_profiles_list(self) -> None:
        lv = self.query_one("#profiles", ListView)
        lv.clear()
        for i, p in enumerate(self.profiles):
            label = p.name + ("  *" if i == self.active_profile_index else "")
            lv.append(ListItem(Label(label), id=f"profile_{i}"))

    def refresh_model_select(self) -> None:
        models: List[str] = []
        for folder in self.global_settings.model_dirs:
            d = Path(folder)
            if d.exists() and d.is_dir():
                try:
                    models.extend(str(x) for x in d.rglob("*.gguf"))
                except Exception:
                    pass
        models = sorted(set(models), key=str.lower)
        select = self.query_one("#p_model_select", Select)
        select.set_options([(m, m) for m in models] if models else [("Aucun modele detecte", "")])

    def refresh_model_dirs_select(self) -> None:
        select = self.query_one("#g_del_dir", Select)
        dirs = self.global_settings.model_dirs
        select.set_options([(d, d) for d in dirs] if dirs else [("Aucun dossier", "")])

    def load_profile_to_ui(self) -> None:
        p = self.profile
        self.query_one("#p_name", Input).value = p.name
        self.query_one("#p_model", Input).value = p.model_path
        self.query_one("#p_host", Input).value = p.host
        self.query_one("#p_port", Input).value = str(p.port)
        self.query_one("#p_ctx", Input).value = str(p.ctx_size)
        self.query_one("#p_threads", Input).value = str(p.threads)
        self.query_one("#p_gpu", Input).value = str(p.n_gpu_layers)
        self.query_one("#p_temp", Input).value = str(p.temp)
        self.query_one("#p_top_p", Input).value = str(p.top_p)
        self.query_one("#p_batch", Input).value = str(p.batch_size)
        self.query_one("#p_embeddings", Switch).value = p.embeddings
        self.query_one("#p_flash", Select).value = p.flash_attn_mode
        self.query_one("#p_extra", Input).value = p.extra_args

    def save_ui_to_profile(self) -> None:
        p = self.profile
        p.name = self.query_one("#p_name", Input).value.strip() or "default"
        p.model_path = self.query_one("#p_model", Input).value.strip()
        p.host = self.query_one("#p_host", Input).value.strip() or "127.0.0.1"
        p.port = int(self.query_one("#p_port", Input).value.strip() or "8080")
        p.ctx_size = int(self.query_one("#p_ctx", Input).value.strip() or "4096")
        p.threads = int(self.query_one("#p_threads", Input).value.strip() or "8")
        p.n_gpu_layers = int(self.query_one("#p_gpu", Input).value.strip() or "0")
        p.temp = float(self.query_one("#p_temp", Input).value.strip() or "0.7")
        p.top_p = float(self.query_one("#p_top_p", Input).value.strip() or "0.95")
        p.batch_size = int(self.query_one("#p_batch", Input).value.strip() or "512")
        p.embeddings = self.query_one("#p_embeddings", Switch).value
        p.flash_attn_mode = str(self.query_one("#p_flash", Select).value or "off")
        p.extra_args = self.query_one("#p_extra", Input).value.strip()

    def load_global_to_ui(self) -> None:
        self.query_one("#g_server", Input).value = self.global_settings.llama_server_path

    def build_command(self) -> List[str]:
        self.save_ui_to_profile()
        exe = resolve_llama_server_path(self.query_one("#g_server", Input).value)
        if not str(exe):
            raise RuntimeError("Definis le chemin de llama-server.exe")
        if not exe.exists():
            raise RuntimeError("llama-server.exe introuvable")
        if exe.suffix.lower() != ".exe":
            raise RuntimeError("Le chemin doit pointer vers llama-server.exe")
        p = self.profile
        if not p.model_path or not Path(p.model_path).exists():
            raise RuntimeError("Modele GGUF introuvable")

        cmd = [
            str(exe),
            "--model",
            p.model_path,
            "--host",
            p.host,
            "--port",
            str(p.port),
            "--ctx-size",
            str(p.ctx_size),
            "--threads",
            str(p.threads),
            "--n-gpu-layers",
            str(p.n_gpu_layers),
            "--temp",
            str(p.temp),
            "--top-p",
            str(p.top_p),
            "--batch-size",
            str(p.batch_size),
            "--flash-attn",
            p.flash_attn_mode,
        ]
        if p.embeddings:
            cmd.append("--embeddings")
        if p.extra_args:
            cmd.extend(shlex.split(p.extra_args, posix=False))
        return cmd

    def action_quit(self) -> None:
        save_profiles(self.profiles)
        save_global(self.global_settings)
        self.exit()

    def on_list_view_selected(self, event: ListView.Selected) -> None:
        if event.list_view.id != "profiles":
            return
        idx = event.list_view.index
        if idx is None or idx < 0 or idx >= len(self.profiles):
            return
        self.active_profile_index = idx
        self.load_profile_to_ui()
        self.refresh_profiles_list()
        self.set_status(f"Profil actif: {self.profile.name}")

    def on_select_changed(self, event: Select.Changed) -> None:
        if event.select.id == "p_model_select" and event.value:
            self.query_one("#p_model", Input).value = str(event.value)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id
        try:
            if bid == "profile_new":
                self.profiles.append(Profile(name=f"profil_{len(self.profiles)+1}"))
                self.active_profile_index = len(self.profiles) - 1
                self.refresh_profiles_list()
                self.load_profile_to_ui()
                self.set_status("Nouveau profil cree.")
            elif bid == "profile_delete":
                if len(self.profiles) == 1:
                    self.set_status("Impossible: un profil minimum.")
                    return
                del self.profiles[self.active_profile_index]
                self.active_profile_index = max(0, self.active_profile_index - 1)
                self.refresh_profiles_list()
                self.load_profile_to_ui()
                self.set_status("Profil supprime.")
            elif bid == "save_profile":
                self.save_ui_to_profile()
                save_profiles(self.profiles)
                self.refresh_profiles_list()
                self.set_status("Profil sauvegarde.")
            elif bid == "save_global":
                self.global_settings.llama_server_path = self.query_one("#g_server", Input).value.strip()
                save_global(self.global_settings)
                self.refresh_model_select()
                self.refresh_model_dirs_select()
                self.set_status("Reglages globaux sauvegardes.")
            elif bid == "g_add_dir_btn":
                val = self.query_one("#g_add_dir", Input).value.strip()
                if not val:
                    self.set_status("Saisis un dossier.")
                    return
                self.global_settings.model_dirs.append(val)
                self.query_one("#g_add_dir", Input).value = ""
                self.refresh_model_select()
                self.refresh_model_dirs_select()
                self.set_status("Dossier modele ajoute.")
            elif bid == "g_del_dir_btn":
                val = self.query_one("#g_del_dir", Select).value
                if not val:
                    self.set_status("Aucun dossier selectionne.")
                    return
                self.global_settings.model_dirs = [d for d in self.global_settings.model_dirs if d != val]
                self.refresh_model_select()
                self.refresh_model_dirs_select()
                self.set_status("Dossier modele supprime.")
            elif bid == "launch":
                self.launch_server()
            elif bid == "stop":
                self.stop_server()
            elif bid == "restart":
                self.stop_server()
                self.launch_server()
            elif bid == "log_stdout":
                self.active_log_file = "stdout"
                self.logs_following = False
                self.render_log_snapshot()
            elif bid == "log_stderr":
                self.active_log_file = "stderr"
                self.logs_following = False
                self.render_log_snapshot()
            elif bid == "log_tail":
                self.logs_following = not self.logs_following
                self._last_log_size = 0
                self.set_status("Live tail ON." if self.logs_following else "Live tail OFF.")
        except Exception as exc:
            self.set_status(f"Erreur: {exc}")

    def log_widget(self) -> Log:
        return self.query_one("#logs", Log)

    def selected_log_path(self) -> Path:
        return LOG_OUT if self.active_log_file == "stdout" else LOG_ERR

    def render_log_snapshot(self) -> None:
        log = self.log_widget()
        path = self.selected_log_path()
        log.clear()
        if not path.exists():
            log.write_line("(fichier absent)")
            return
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()[-120:]
        for line in lines:
            log.write_line(line)
        self.set_status(f"Affichage {path.name}")

    def tick_tail(self) -> None:
        if not self.logs_following:
            return
        path = self.selected_log_path()
        log = self.log_widget()
        if not path.exists():
            return
        data = path.read_text(encoding="utf-8", errors="replace")
        current = len(data)
        if self._last_log_size > current:
            self._last_log_size = 0
            log.clear()
        if current <= self._last_log_size:
            return
        chunk = data[self._last_log_size :]
        self._last_log_size = current
        for line in chunk.splitlines():
            log.write_line(line)

    def launch_server(self) -> None:
        ensure_state()
        cmd = self.build_command()
        self.global_settings.llama_server_path = self.query_one("#g_server", Input).value.strip()
        save_profiles(self.profiles)
        save_global(self.global_settings)
        if LOG_OUT.exists():
            LOG_OUT.unlink()
        if LOG_ERR.exists():
            LOG_ERR.unlink()
        with LOG_OUT.open("w", encoding="utf-8") as out, LOG_ERR.open("w", encoding="utf-8") as err:
            p = subprocess.Popen(
                cmd,
                stdout=out,
                stderr=err,
                stdin=subprocess.DEVNULL,
                creationflags=0x08000000 | 0x00000200,
                cwd=str(APP_DIR),
            )
        PID_FILE.write_text(str(p.pid), encoding="utf-8")
        self._last_log_size = 0
        self.set_status(f"Lance PID {p.pid}")

    def stop_server(self) -> None:
        pid = pid_value()
        if pid <= 0:
            self.set_status("Aucun PID enregistre.")
            return
        subprocess.run(
            ["taskkill", "/PID", str(pid), "/F", "/T"],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if PID_FILE.exists():
            PID_FILE.unlink()
        self.set_status(f"Stop demande pour PID {pid}.")


if __name__ == "__main__":
    LlamaLauncherApp().run()
