"""LLama Launcher TUI application — package-local implementation."""
import re
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

from llama_launcher.api import LlamaLauncherService
from llama_launcher.models import GlobalSettings, LlamaOption, Profile


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
        self.service = LlamaLauncherService()
        self.global_settings = self.service.load_global()
        self.profiles = self.service.load_profiles()
        self._last_log_size = 0
        self._last_log_marker = ""
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
        if not server_input.value.strip() and self.service.default_server_path.exists():
            server_input.value = str(self.service.default_server_path)
            self.global_settings.llama_server_path = str(self.service.default_server_path)
            self.service.save_global(self.global_settings)
        self.refresh_model_select()
        self.refresh_model_dirs_select()
        self.load_advanced_from_server(auto=True)
        self.query_one("#adv_desc", Static).visible = False
        self.query_one("#logs", Log).visible = True
        running, existing_pid = self.service.is_server_running()
        if existing_pid > 0 and running:
            self.session_has_server_output = True
            self.render_log_snapshot()
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
        models = self.service.discover_models(self.global_settings.model_dirs)
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
        exe = self.query_one("#g_server", Input).value
        if not exe or not exe.strip():
            if not auto:
                self.set_status("Chemin llama-server invalide.")
            self.llama_options = {}
            self.refresh_advanced_table()
            self.refresh_advanced_favorites()
            return
        try:
            opts = self.service.load_options(exe)
        except RuntimeError as exc:
            if "Chemin llama-server invalide" in str(exc):
                self.set_status("Chemin llama-server invalide.")
            else:
                self.set_status(f"Echec chargement des options avancees: {exc}")
            self.llama_options = {}
            self.refresh_advanced_table()
            self.refresh_advanced_favorites()
            return
        if not opts:
            if not auto:
                self.set_status("Echec chargement des options avancees.")
            self.llama_options = {}
            self.refresh_advanced_table()
            self.refresh_advanced_favorites()
            return
        self.llama_options = opts
        self.refresh_advanced_table()
        self.refresh_advanced_favorites()
        self.query_one("#adv_desc", Static).visible = False
        if not auto:
            self.set_status(f"{len(opts)} options chargees depuis --help.")

    def canonical_adv_key(self, raw_key: str) -> str:
        return self.service.canonical_adv_key(raw_key, self.llama_options)

    def fav_widget_suffix(self, raw_key: str) -> str:
        return re.sub(r"[^a-zA-Z0-9_]", "_", raw_key)

    def favorite_string_value(self, raw_key: str, key: str, opt: LlamaOption | None) -> str | None:
        return self.service.favorite_string_value(raw_key, key, opt, self.profile)

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
            val = self.favorite_string_value(raw_key, key, opt) or ""
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
                value = self.query_one(f"#fav_val_{suffix}", Input).value.strip()
                if value:
                    p.advanced_values[raw_key] = value
                else:
                    p.advanced_values.pop(raw_key, None)
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

        self.service.save_profiles(self.profiles)
        self.refresh_advanced_favorites()
        self.set_status("Option avancee activee/desactivee.")

    def build_command(self) -> List[str]:
        self.save_ui_to_profile()
        exe_input = self.query_one("#g_server", Input).value
        p = self.profile
        return self.service.build_command(p, exe_input, self.llama_options)

    def action_quit(self) -> None:
        self.service.save_profiles(self.profiles)
        self.service.save_global(self.global_settings)
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
                self.service.save_profiles(self.profiles)
                self.refresh_profiles_list()
                self.refresh_advanced_favorites()
                self.set_status("Profil sauvegarde.")
            elif bid == "save_global":
                self.global_settings.llama_server_path = self.query_one("#g_server", Input).value.strip()
                self.service.save_global(self.global_settings)
                self.refresh_model_select()
                self.refresh_model_dirs_select()
                self.load_advanced_from_server(auto=True)
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
        return self.service.log_out_path

    def render_log_snapshot(self) -> None:
        log = self.log_widget()
        log.clear()
        if not self.session_has_server_output:
            return
        path = self.selected_log_path()
        if not path.exists():
            return
        data = path.read_text(encoding="utf-8", errors="replace")
        lines = data.splitlines()[-120:]
        for line in lines:
            log.write_line(line)
        self._last_log_size = len(data)
        marker_start = max(0, len(data) - 64)
        self._last_log_marker = data[marker_start:len(data)]
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
        chunk, new_size, reset_required, new_marker = self.service.tail_log(
            self._last_log_size, self._last_log_marker
        )
        if reset_required:
            log.clear()
            for line in chunk.splitlines():
                log.write_line(line)
            self._last_log_size = new_size
            self._last_log_marker = new_marker
            return
        self._last_log_size = new_size
        self._last_log_marker = new_marker
        if not chunk:
            return
        for line in chunk.splitlines():
            log.write_line(line)

    def tick_monitoring(self) -> None:
        mon = self.query_one("#monitoring", Static)
        mon.update(self.service.build_monitoring_text())

    def launch_server(self) -> None:
        running, pid = self.service.is_server_running()
        if running and pid > 0:
            self.set_status(f"llama-server deja en cours (PID {pid}). Stop avant relance.")
            return
        cmd = self.build_command()
        exe_path = self.query_one("#g_server", Input).value.strip()
        child_pid = self.service.launch(cmd, exe_path)
        self.service.save_profiles(self.profiles)
        self.session_has_server_output = True
        self._last_log_size = 0
        self._last_log_marker = ""
        self.log_widget().clear()
        self.set_status(f"Lance PID {child_pid}")

    def stop_server(self) -> None:
        running, pid = self.service.is_server_running()
        if pid <= 0:
            self.set_status("Aucun PID enregistre.")
            return
        if not running:
            self.service.stop()
            self.set_status("PID obsolete nettoye (processus deja arrete).")
            return
        stopped_pid = self.service.stop()
        self.set_status(f"Stop demande pour PID {stopped_pid}.")


if __name__ == "__main__":
    LlamaLauncherApp().run()
