"""Data models for LLama Launcher."""
from dataclasses import dataclass, field
from typing import Dict, List


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
    api_host: str = "127.0.0.1"
    api_port: int = 0

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
    top_k: int = 40
    min_p: float = 0.05
    presence_penalty: float = 0.0
    np: int = 1
    batch_size: int = 512
    enable_mtp: bool = False
    spec_draft_n_max: int = 2
    embeddings: bool = False
    flash_attn_mode: str = "off"
    kv_cache_type: str = "f16"
    extra_args: str = ""
    advanced_values: Dict[str, str] = field(default_factory=dict)
    advanced_modes: Dict[str, str] = field(default_factory=dict)
    advanced_favorites: List[str] = field(default_factory=list)
