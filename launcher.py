import json
import re
import shlex
import subprocess
import ctypes
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List

try:
    from textual.app import App, ComposeResult
    from textual.containers import Horizontal, Vertical
    from textual.reactive import reactive
    from textual.widgets import Button, Checkbox, Footer, Header, Input, Label, ListItem, ListView, Log, Select, Static, Switch
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
DEFAULT_LLAMA_SERVER = Path(r"C:\llama-cpp\llama-server.exe")


@dataclass
class LlamaOption:
    key: str
    aliases: List[str]
    arity: int
    default: str
    description: str
    positive_flag: str
    negative_flag: str


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
    flash_attn_mode: str = "off"
    kv_cache_type: str = "f16"
    extra_args: str = ""
    advanced_values: Dict[str, str] = field(default_factory=dict)
    advanced_modes: Dict[str, str] = field(default_factory=dict)
    advanced_favorites: List[str] = field(default_factory=list)


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
            item.setdefault("advanced_values", {})
            item.setdefault("advanced_favorites", [])
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
    if not str(raw).strip():
        return DEFAULT_LLAMA_SERVER
    return p


def parse_help_options(help_text: str) -> Dict[str, LlamaOption]:
    lines = help_text.splitlines()
    sections: List[List[str]] = []
    current: List[str] = []
    for line in lines:
        if re.match(r"^\s*-[^-\s]|^\s*--", line):
            if current:
                sections.append(current)
            current = [line]
        elif current and line.strip():
            current.append(line)
        else:
            if current:
                sections.append(current)
                current = []
    if current:
        sections.append(current)

    options: Dict[str, LlamaOption] = {}
    for block in sections:
        first = block[0].strip()
        parts = re.split(r"\s{2,}", first, maxsplit=1)
        names_raw = parts[0].strip()
        desc = parts[1].strip() if len(parts) > 1 else ""
        if len(block) > 1:
            desc += " " + " ".join(x.strip() for x in block[1:])
        desc = re.sub(r"\s+", " ", desc).strip()

        alias_specs = [x.strip() for x in names_raw.split(",") if x.strip()]
        aliases: List[str] = []
        arity = 0
        for spec in alias_specs:
            chunks = spec.split()
            if not chunks:
                continue
            flag = chunks[0]
            aliases.append(flag)
            if len(chunks) > 1:
                arity = max(arity, len(chunks) - 1)

        if not aliases:
            continue

        long_aliases = [a for a in aliases if a.startswith("--")]
        non_no = [a for a in long_aliases if not a.startswith("--no-")]
        key = (non_no[0] if non_no else (long_aliases[0] if long_aliases else aliases[0]))
        positive = non_no[0] if non_no else ""
        negative = ""
        if positive:
            neg_candidate = "--no-" + positive[2:]
            if neg_candidate in aliases:
                negative = neg_candidate

        default_match = re.search(r"\(default:\s*([^\)]+)\)", desc)
        default_value = default_match.group(1).strip() if default_match else ""

        options[key] = LlamaOption(
            key=key,
            aliases=aliases,
            arity=arity,
            default=default_value,
            description=desc,
            positive_flag=positive,
            negative_flag=negative,
        )
    return options


def load_options_from_exe(exe_path: Path) -> Dict[str, LlamaOption]:
    try:
        proc = subprocess.run(
            [str(exe_path), "--help"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        txt = (proc.stdout or "") + "\n" + (proc.stderr or "")
        return parse_help_options(txt)
    except Exception:
        return {}


def pid_value() -> int:
    if not PID_FILE.exists():
        return 0
    try:
        return int(PID_FILE.read_text(encoding="utf-8").strip())
    except Exception:
        return 0


def is_process_running(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        proc = subprocess.run(
            ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        out = (proc.stdout or "").strip()
        if not out:
            return False
        if "No tasks are running" in out:
            return False
        return f'"{pid}"' in out
    except Exception:
        return False


class LlamaLauncherApp(App):
    CSS = """
    #main { layout: vertical; height: 1fr; }
    #root { layout: horizontal; height: 1fr; }
    #left { width: 30; padding: 0 1; border: round #666666; }
    #center { width: 1fr; padding: 0 1; border: round #666666; overflow-y: auto; }
    #right { width: 56; padding: 0 1; border: round #666666; }
    #console_panel { height: 14; padding: 0; border: none; margin-top: 0; position: relative; }
    #console_fullscreen_btn {
        width: 5;
        height: 1;
        min-width: 5;
        position: absolute;
        layer: overlay;
        offset: 1 1;
        content-align: center middle;
        text-align: center;
        border: round #666666;
        background: #111111 85%;
    }
    #logs { height: 1fr; border: round #444444; margin-top: 0; }
    .row { height: auto; margin: 0; }
    .title { text-style: bold; margin: 0 0 1 0; }
    .pair { height: 3; }
    .pair Label { width: 14; }
    #adv_search { width: 1fr; }
    #status { height: 3; border: round #555555; padding: 0 1; margin-top: 1; }
    #profile_block { height: auto; border: round #444444; padding: 0 1; }
    #adv_desc { height: 8; border: round #444444; padding: 0 1; }
    #adv_table { height: 12; border: round #444444; padding: 0 1; }
    #monitoring { height: 4; border: round #444444; padding: 0 1; margin: 1 0; }
    .adv-row { height: 1; }
    .adv-opt { width: 1fr; }
    .adv-opt-btn { width: 1fr; content-align: left middle; text-align: left; border: none; padding: 0; }
    .center-opt-btn { width: 14; content-align: left middle; text-align: left; border: none; padding: 0; }
    .adv-fav-col { width: 3; content-align: right middle; }
    .adv-fav-col Checkbox { border: none; padding: 0; margin: 0; }
    #adv_favorites_fields { height: auto; border: round #444444; padding: 0 1; margin-bottom: 1; }
    """

    active_profile_index = reactive(0)
    logs_following = reactive(True)
    console_fullscreen = reactive(False)

    def __init__(self) -> None:
        super().__init__()
        self.global_settings = load_global()
        self.profiles = load_profiles()
        self._last_log_size = 0
        self.llama_options: Dict[str, LlamaOption] = {}
        self._fav_row_keys: List[str] = []
        self.adv_search_text = ""
        self.adv_table_switch_to_key: Dict[str, str] = {}
        self.adv_table_button_to_key: Dict[str, str] = {}
        self.center_fav_button_to_key: Dict[str, str] = {}
        self.session_has_server_output = False

    @property
    def profile(self) -> Profile:
        return self.profiles[self.active_profile_index]

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with Vertical(id="main"):
            with Horizontal(id="root"):
                with Vertical(id="left"):
                    yield Label("Profils", classes="title")
                    yield ListView(id="profiles")
                    yield Button("Nouveau profil", id="profile_new")
                    yield Button("Supprimer profil", id="profile_delete")
                    yield Static("", id="status")
                with Vertical(id="center"):
                    yield Label("Reglages profil", classes="title")
                    with Vertical(id="profile_block"):
                        yield Horizontal(Label("--alias"), Input(placeholder="profil", id="p_name"), classes="pair")
                        yield Horizontal(Label("-m/--model"), Input(placeholder=".gguf", id="p_model"), classes="pair")
                        yield Select([], prompt="Modele detecte", id="p_model_select")
                        yield Horizontal(Label("--host"), Input(value="127.0.0.1", id="p_host"), classes="pair")
                        yield Horizontal(Label("--port"), Input(value="8080", id="p_port"), classes="pair")
                        yield Horizontal(Label("-c/--ctx-size"), Input(value="4096", id="p_ctx"), classes="pair")
                        yield Horizontal(Label("-t/--threads"), Input(value="8", id="p_threads"), classes="pair")
                        yield Horizontal(Label("-ngl/--n-gpu-layers"), Input(value="0", id="p_gpu"), classes="pair")
                        yield Horizontal(Label("--temp"), Input(value="0.7", id="p_temp"), classes="pair")
                        yield Horizontal(Label("--top-p"), Input(value="0.95", id="p_top_p"), classes="pair")
                        yield Horizontal(Label("-b/--batch-size"), Input(value="512", id="p_batch"), classes="pair")
                        yield Horizontal(Label("--embeddings"), Switch(id="p_embeddings"), classes="row")
                        yield Horizontal(Label("-fa/--flash-attn"), Select([("off", "off"), ("on", "on"), ("auto", "auto")], id="p_flash"), classes="row")
                        yield Horizontal(Label("-ctk/-ctv"), Select([("f16", "f16"), ("q8_0", "q8_0"), ("q4_0", "q4_0")], id="p_kv_cache"), classes="row")
                        yield Horizontal(Label("extra args"), Input(placeholder="Extra args", id="p_extra"), classes="pair")
                        yield Label("Options avancees actives", classes="title")
                        with Vertical(id="adv_favorites_fields"):
                            yield Static("(aucune option active)")
                    yield Horizontal(Button("Sauver profil", id="save_profile"), classes="row")
                with Vertical(id="right"):
                    yield Horizontal(Button("Launch", id="launch"), Button("Stop", id="stop"), Button("Restart", id="restart"), classes="row")
                    yield Static("RAM: N/A\nVRAM: N/A", id="monitoring")
                    yield Horizontal(Label("Server"), Input(placeholder="llama-server.exe", id="g_server"), classes="pair")
                    yield Label("Configuration Avancee", classes="title")
                    yield Horizontal(Label("Recherche"), Input(placeholder="rechercher une option...", id="adv_search"), classes="row")
                    with Vertical(id="adv_table"):
                        yield Horizontal(Label("Option", classes="adv-opt"), Label("Actif", classes="adv-fav-col"), classes="adv-row")
                    yield Static("Aucune option chargee.", id="adv_desc")
                    yield Horizontal(Label("Add dir"), Input(placeholder="dossier modeles", id="g_add_dir"), Button("+", id="g_add_dir_btn"), classes="pair")
                    yield Select([], prompt="Supprimer dossier modeles", id="g_del_dir")
                    yield Button("Supprimer dossier", id="g_del_dir_btn")
                    yield Button("Sauver global", id="save_global")
            with Vertical(id="console_panel"):
                yield Log(id="logs")
                yield Button("FS", id="console_fullscreen_btn")
        yield Footer()

    def on_mount(self) -> None:
        self.refresh_profiles_list()
        self.load_profile_to_ui()
        self.load_global_to_ui()
        server_input = self.query_one("#g_server", Input)
        if not server_input.value.strip() and DEFAULT_LLAMA_SERVER.exists():
            server_input.value = str(DEFAULT_LLAMA_SERVER)
            self.global_settings.llama_server_path = str(DEFAULT_LLAMA_SERVER)
            save_global(self.global_settings)
        self.refresh_model_select()
        self.refresh_model_dirs_select()
        self.load_advanced_from_server(auto=True)
        self.query_one("#adv_desc", Static).visible = False
        self.query_one("#logs", Log).visible = True
        existing_pid = pid_value()
        if existing_pid > 0 and is_process_running(existing_pid):
            self.session_has_server_output = True
            self._last_log_size = 0
            self.set_status(f"Attache a llama-server deja actif (PID {existing_pid}).")
        else:
            self.set_status("Pret.")
        self.position_console_button()
        self.set_interval(0.2, self.tick_tail)
        self.set_interval(1.0, self.tick_monitoring)
        self.tick_monitoring()

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

    def filtered_advanced_options(self) -> List[LlamaOption]:
        items = sorted(self.llama_options.values(), key=lambda o: o.key)
        q = self.adv_search_text.strip().lower()
        if not q:
            return items
        return [
            o for o in items
            if q in o.key.lower()
            or any(q in a.lower() for a in o.aliases)
            or q in o.description.lower()
        ]

    def refresh_advanced_table(self) -> None:
        table = self.query_one("#adv_table", Vertical)
        table.remove_children()
        table.mount(Horizontal(Label("Option", classes="adv-opt"), Label("Actif", classes="adv-fav-col"), classes="adv-row"))
        self.adv_table_switch_to_key = {}
        self.adv_table_button_to_key = {}
        p = self.profile
        favorites_canon = {self.canonical_adv_key(x) for x in p.advanced_favorites}
        for opt in self.filtered_advanced_options():
            short_alias = next((a for a in opt.aliases if a.startswith("-") and not a.startswith("--")), "")
            label = f"{short_alias} / {opt.key}" if short_alias else opt.key
            sid = f"adv_tbl_fav_{self.fav_widget_suffix(opt.key)}"
            bid = f"adv_tbl_opt_{self.fav_widget_suffix(opt.key)}"
            cb = Checkbox("", id=sid)
            cb.value = opt.key in favorites_canon
            self.adv_table_switch_to_key[sid] = opt.key
            self.adv_table_button_to_key[bid] = opt.key
            table.mount(
                Horizontal(
                    Button(label, id=bid, classes="adv-opt-btn"),
                    Vertical(cb, classes="adv-fav-col"),
                    classes="adv-row",
                )
            )

    def load_advanced_from_server(self, auto: bool = False) -> None:
        exe = resolve_llama_server_path(self.query_one("#g_server", Input).value)
        if not str(exe) or not exe.exists():
            if not auto:
                self.set_status("Chemin llama-server invalide.")
            return
        opts = load_options_from_exe(exe)
        if not opts:
            self.set_status("Echec chargement des options avancees.")
            return
        self.llama_options = opts
        self.refresh_advanced_table()
        self.refresh_advanced_favorites()
        self.query_one("#adv_desc", Static).visible = False
        if not auto:
            self.set_status(f"{len(opts)} options chargees depuis --help.")

    def canonical_adv_key(self, raw_key: str) -> str:
        if raw_key in self.llama_options:
            return raw_key
        for key, opt in self.llama_options.items():
            if raw_key in opt.aliases:
                return key
        return raw_key

    def fav_widget_suffix(self, raw_key: str) -> str:
        return re.sub(r"[^a-zA-Z0-9_]", "_", raw_key)

    def favorite_string_value(self, raw_key: str, key: str, opt: LlamaOption | None) -> str:
        p = self.profile
        if raw_key in p.advanced_values:
            return p.advanced_values.get(raw_key, "")
        if key in p.advanced_values:
            return p.advanced_values.get(key, "")
        legacy_modes = getattr(p, "advanced_modes", {}) or {}
        mode = legacy_modes.get(raw_key, legacy_modes.get(key, "default"))
        if mode == "on":
            return ""
        if mode == "off" and opt and opt.negative_flag:
            return opt.negative_flag
        return ""

    def refresh_advanced_favorites(self) -> None:
        p = self.profile
        box = self.query_one("#adv_favorites_fields", Vertical)
        box.remove_children()
        self._fav_row_keys = list(p.advanced_favorites)
        self.center_fav_button_to_key = {}

        if not self._fav_row_keys:
            box.mount(Static("(aucune option active)"))
            return

        for raw_key in p.advanced_favorites:
            key = self.canonical_adv_key(raw_key)
            opt = self.llama_options.get(key)
            val = self.favorite_string_value(raw_key, key, opt)
            bid = f"center_fav_opt_{self.fav_widget_suffix(raw_key)}"
            self.center_fav_button_to_key[bid] = key
            if not opt:
                box.mount(
                    Horizontal(
                        Button(raw_key, id=bid, classes="center-opt-btn"),
                        Input(value=val, placeholder="arguments string", id=f"fav_val_{self.fav_widget_suffix(raw_key)}"),
                        classes="pair",
                    )
                )
                continue
            short_alias = next((a for a in opt.aliases if a.startswith("-") and not a.startswith("--")), "")
            label = f"{short_alias} / {key}" if short_alias else key
            box.mount(
                Horizontal(
                    Button(label, id=bid, classes="center-opt-btn"),
                    Input(
                        value=val,
                        placeholder=f"arguments string (defaut: {opt.default or 'n/a'})",
                        id=f"fav_val_{self.fav_widget_suffix(raw_key)}",
                    ),
                    classes="pair",
                )
            )

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
        self.query_one("#p_kv_cache", Select).value = p.kv_cache_type
        self.query_one("#p_extra", Input).value = p.extra_args
        self.refresh_advanced_favorites()
        if self.llama_options:
            self.refresh_advanced_table()

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
        p.kv_cache_type = str(self.query_one("#p_kv_cache", Select).value or "f16")
        p.extra_args = self.query_one("#p_extra", Input).value.strip()
        for raw_key in self._fav_row_keys:
            suffix = self.fav_widget_suffix(raw_key)
            try:
                p.advanced_values[raw_key] = self.query_one(f"#fav_val_{suffix}", Input).value.strip()
            except Exception:
                pass

    def load_global_to_ui(self) -> None:
        self.query_one("#g_server", Input).value = self.global_settings.llama_server_path

    def set_option_favorite(self, key: str, fav: bool) -> None:
        key = self.canonical_adv_key(key)
        if not key:
            return
        p = self.profile
        if fav:
            favorites_canon = {self.canonical_adv_key(x) for x in p.advanced_favorites}
            if key not in favorites_canon:
                p.advanced_favorites.append(key)
        if not fav:
            p.advanced_favorites = [
                x for x in p.advanced_favorites if self.canonical_adv_key(x) != key
            ]

        save_profiles(self.profiles)
        self.refresh_advanced_favorites()
        self.set_status("Option avancee activee/desactivee.")

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
            "--model", p.model_path,
            "--host", p.host,
            "--port", str(p.port),
            "--ctx-size", str(p.ctx_size),
            "--threads", str(p.threads),
            "--n-gpu-layers", str(p.n_gpu_layers),
            "--temp", str(p.temp),
            "--top-p", str(p.top_p),
            "--batch-size", str(p.batch_size),
            "--flash-attn", p.flash_attn_mode,
            "--cache-type-k", p.kv_cache_type,
            "--cache-type-v", p.kv_cache_type,
        ]
        if p.embeddings:
            cmd.append("--embeddings")

        for raw_key in p.advanced_favorites:
            ckey = self.canonical_adv_key(raw_key)
            opt = self.llama_options.get(ckey)
            val = self.favorite_string_value(raw_key, ckey, opt).strip()
            cmd.append(ckey)
            if val:
                cmd.extend(shlex.split(val, posix=False))

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

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "adv_search":
            self.adv_search_text = event.input.value
            self.refresh_advanced_table()
            self.query_one("#adv_desc", Static).visible = False

    def on_switch_changed(self, event: Switch.Changed) -> None:
        return

    def on_checkbox_changed(self, event: Checkbox.Changed) -> None:
        sid = event.checkbox.id or ""
        if sid.startswith("adv_tbl_fav_"):
            key = self.adv_table_switch_to_key.get(sid, "")
            if key:
                self.set_option_favorite(key, event.value)
                opt = self.llama_options.get(key)
                if opt:
                    desc = self.query_one("#adv_desc", Static)
                    desc.update(
                        f"Option: {key}\nAliases: {', '.join(opt.aliases)}\nDefaut: {opt.default or 'n/a'}\n{opt.description}"
                    )
                    desc.visible = True

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id
        try:
            if bid and bid.startswith("adv_tbl_opt_"):
                key = self.adv_table_button_to_key.get(bid, "")
                opt = self.llama_options.get(key)
                if opt:
                    desc = self.query_one("#adv_desc", Static)
                    desc.update(
                        f"Option: {key}\nAliases: {', '.join(opt.aliases)}\nDefaut: {opt.default or 'n/a'}\n{opt.description}"
                    )
                    desc.visible = True
                return
            if bid and bid.startswith("center_fav_opt_"):
                key = self.center_fav_button_to_key.get(bid, "")
                opt = self.llama_options.get(key)
                if opt:
                    desc = self.query_one("#adv_desc", Static)
                    desc.update(
                        f"Option: {key}\nAliases: {', '.join(opt.aliases)}\nDefaut: {opt.default or 'n/a'}\n{opt.description}"
                    )
                    desc.visible = True
                return
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
                self.refresh_advanced_favorites()
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
            elif bid == "console_fullscreen_btn":
                self.toggle_console_fullscreen()
        except Exception as exc:
            self.set_status(f"Erreur: {exc}")

    def on_resize(self) -> None:
        self.position_console_button()

    def position_console_button(self) -> None:
        try:
            panel = self.query_one("#console_panel", Vertical)
            btn = self.query_one("#console_fullscreen_btn", Button)
            width = panel.size.width or 0
            btn_w = btn.size.width or 5
            x = max(1, width - btn_w - 2)
            btn.styles.offset = (x, 1)
        except Exception:
            pass

    def toggle_console_fullscreen(self) -> None:
        self.console_fullscreen = not self.console_fullscreen
        root = self.query_one("#root", Horizontal)
        console_panel = self.query_one("#console_panel", Vertical)
        btn = self.query_one("#console_fullscreen_btn", Button)
        if self.console_fullscreen:
            root.styles.display = "none"
            console_panel.styles.height = "1fr"
            btn.label = "EX"
            self.position_console_button()
            self.set_status("Console en plein ecran.")
            return
        root.styles.display = "block"
        console_panel.styles.height = 14
        btn.label = "FS"
        self.position_console_button()
        self.set_status("Retour vue normale.")

    def log_widget(self) -> Log:
        return self.query_one("#logs", Log)

    def selected_log_path(self) -> Path:
        return LOG_OUT

    def render_log_snapshot(self) -> None:
        log = self.log_widget()
        log.clear()
        if not self.session_has_server_output:
            return
        path = self.selected_log_path()
        if not path.exists():
            return
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()[-120:]
        for line in lines:
            log.write_line(line)
        self.set_status(f"Affichage live {path.name}")

    def tick_tail(self) -> None:
        if not self.logs_following:
            return
        if not self.session_has_server_output:
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

    def bytes_to_gb(self, value: int) -> str:
        gb = value / (1024 ** 3)
        return f"{gb:.1f}GB"

    def ram_usage_bytes(self) -> tuple[int, int]:
        class MemoryStatusEx(ctypes.Structure):
            _fields_ = [
                ("dwLength", ctypes.c_ulong),
                ("dwMemoryLoad", ctypes.c_ulong),
                ("ullTotalPhys", ctypes.c_ulonglong),
                ("ullAvailPhys", ctypes.c_ulonglong),
                ("ullTotalPageFile", ctypes.c_ulonglong),
                ("ullAvailPageFile", ctypes.c_ulonglong),
                ("ullTotalVirtual", ctypes.c_ulonglong),
                ("ullAvailVirtual", ctypes.c_ulonglong),
                ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
            ]

        status = MemoryStatusEx()
        status.dwLength = ctypes.sizeof(MemoryStatusEx)
        if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return (0, 0)
        total = int(status.ullTotalPhys)
        avail = int(status.ullAvailPhys)
        used = max(0, total - avail)
        return (used, total)

    def process_ram_bytes(self, pid: int) -> int:
        if pid <= 0:
            return 0
        try:
            proc = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            out = (proc.stdout or "").strip()
            if not out or "INFO:" in out.upper():
                return 0
            row = [x.strip('"') for x in out.split('","')]
            if len(row) < 5:
                return 0
            mem_field = row[4].replace(",", "").replace(" ", "").replace("K", "").strip()
            kb = int(mem_field) if mem_field.isdigit() else 0
            return kb * 1024
        except Exception:
            return 0

    def gpu_vram_info(self) -> tuple[int, int]:
        # Global GPU VRAM usage across all processes.
        try:
            gpus = subprocess.run(
                [
                    "nvidia-smi",
                    "--query-gpu=memory.used,memory.total",
                    "--format=csv,noheader,nounits",
                ],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            lines = [ln.strip() for ln in (gpus.stdout or "").splitlines() if ln.strip()]
            if not lines:
                return (0, 0)

            used_sum = 0
            total_sum = 0
            for line in lines:
                parts = [x.strip() for x in line.split(",")]
                if len(parts) < 2:
                    continue
                if parts[0].isdigit():
                    used_sum += int(parts[0]) * 1024 * 1024
                if parts[1].isdigit():
                    total_sum += int(parts[1]) * 1024 * 1024

            return (used_sum, total_sum)
        except Exception:
            return (0, 0)

    def tick_monitoring(self) -> None:
        mon = self.query_one("#monitoring", Static)

        used_ram, total_ram = self.ram_usage_bytes()
        ram_line = "RAM: N/A"
        if total_ram > 0:
            ram_line = f"RAM: {self.bytes_to_gb(used_ram)}/{self.bytes_to_gb(total_ram)}"

        used_vram, total_vram = self.gpu_vram_info()
        vram_line = "VRAM: N/A"
        if total_vram > 0:
            vram_line = f"VRAM: {self.bytes_to_gb(used_vram)}/{self.bytes_to_gb(total_vram)}"

        mon.update(f"{ram_line}\n{vram_line}")

    def launch_server(self) -> None:
        ensure_state()
        existing_pid = pid_value()
        if existing_pid > 0 and is_process_running(existing_pid):
            self.set_status(f"llama-server deja en cours (PID {existing_pid}). Stop avant relance.")
            return
        if existing_pid > 0 and PID_FILE.exists():
            PID_FILE.unlink()
        cmd = self.build_command()
        self.global_settings.llama_server_path = self.query_one("#g_server", Input).value.strip()
        save_profiles(self.profiles)
        save_global(self.global_settings)
        if LOG_OUT.exists():
            LOG_OUT.unlink()
        if LOG_ERR.exists():
            LOG_ERR.unlink()
        with LOG_OUT.open("w", encoding="utf-8") as out:
            p = subprocess.Popen(
                cmd,
                stdout=out,
                stderr=subprocess.STDOUT,
                stdin=subprocess.DEVNULL,
                creationflags=0x08000000 | 0x00000200,
                cwd=str(APP_DIR),
            )
        PID_FILE.write_text(str(p.pid), encoding="utf-8")
        self.session_has_server_output = True
        self._last_log_size = 0
        self.log_widget().clear()
        self.set_status(f"Lance PID {p.pid}")

    def stop_server(self) -> None:
        pid = pid_value()
        if pid <= 0:
            self.set_status("Aucun PID enregistre.")
            return
        if not is_process_running(pid):
            if PID_FILE.exists():
                PID_FILE.unlink()
            self.set_status("PID obsolete nettoye (processus deja arrete).")
            return
        subprocess.run(["taskkill", "/PID", str(pid), "/F", "/T"], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if PID_FILE.exists():
            PID_FILE.unlink()
        self.set_status(f"Stop demande pour PID {pid}.")


if __name__ == "__main__":
    LlamaLauncherApp().run()
