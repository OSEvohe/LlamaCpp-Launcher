"""Entry point for the LLama Launcher TUI."""


def main():
    from llama_launcher.ui.app import LlamaLauncherApp
    LlamaLauncherApp().run()


if __name__ == "__main__":
    main()
