from __future__ import annotations

import json
import threading
from pathlib import Path
from typing import Any

from flask import Flask, jsonify, request, send_from_directory
from flask_cors import CORS

try:
    from langdetect import DetectorFactory, LangDetectException, detect

    DetectorFactory.seed = 7
except Exception:
    DetectorFactory = None
    LangDetectException = Exception
    detect = None

try:
    from transformers.models.nllb.tokenization_nllb import FAIRSEQ_LANGUAGE_CODES
except Exception:
    FAIRSEQ_LANGUAGE_CODES = []


ROOT_DIR = Path(__file__).resolve().parents[1]
FRONTEND_DIR = ROOT_DIR / "frontend"
CONFIG_PATH = ROOT_DIR / "config.json"

LANGUAGES = [
    {"code": "auto", "name": "Detect automatically", "native": "Auto", "popular": True},
    {"code": "eng_Latn", "name": "English", "native": "English", "popular": True, "iso": "en"},
    {"code": "fra_Latn", "name": "French", "native": "Francais", "popular": True, "iso": "fr"},
    {"code": "spa_Latn", "name": "Spanish", "native": "Espanol", "popular": True, "iso": "es"},
    {"code": "deu_Latn", "name": "German", "native": "Deutsch", "popular": True, "iso": "de"},
    {"code": "ita_Latn", "name": "Italian", "native": "Italiano", "popular": True, "iso": "it"},
    {"code": "por_Latn", "name": "Portuguese", "native": "Portugues", "popular": True, "iso": "pt"},
    {"code": "nld_Latn", "name": "Dutch", "native": "Nederlands", "iso": "nl"},
    {"code": "pol_Latn", "name": "Polish", "native": "Polski", "iso": "pl"},
    {"code": "rus_Cyrl", "name": "Russian", "native": "Russkiy", "popular": True, "iso": "ru"},
    {"code": "ukr_Cyrl", "name": "Ukrainian", "native": "Ukrainska", "iso": "uk"},
    {"code": "zho_Hans", "name": "Chinese Simplified", "native": "Simplified Chinese", "popular": True, "iso": "zh-cn"},
    {"code": "zho_Hant", "name": "Chinese Traditional", "native": "Traditional Chinese", "iso": "zh-tw"},
    {"code": "jpn_Jpan", "name": "Japanese", "native": "Nihongo", "popular": True, "iso": "ja"},
    {"code": "kor_Hang", "name": "Korean", "native": "Hangugeo", "popular": True, "iso": "ko"},
    {"code": "arb_Arab", "name": "Arabic", "native": "Arabic", "popular": True, "iso": "ar"},
    {"code": "hin_Deva", "name": "Hindi", "native": "Hindi", "popular": True, "iso": "hi"},
    {"code": "ben_Beng", "name": "Bengali", "native": "Bangla", "iso": "bn"},
    {"code": "tur_Latn", "name": "Turkish", "native": "Turkce", "iso": "tr"},
    {"code": "vie_Latn", "name": "Vietnamese", "native": "Tieng Viet", "iso": "vi"},
    {"code": "tha_Thai", "name": "Thai", "native": "Thai", "iso": "th"},
    {"code": "ind_Latn", "name": "Indonesian", "native": "Bahasa Indonesia", "iso": "id"},
    {"code": "ron_Latn", "name": "Romanian", "native": "Romana", "iso": "ro"},
    {"code": "ces_Latn", "name": "Czech", "native": "Cestina", "iso": "cs"},
    {"code": "ell_Grek", "name": "Greek", "native": "Ellinika", "iso": "el"},
    {"code": "swe_Latn", "name": "Swedish", "native": "Svenska", "iso": "sv"},
    {"code": "fin_Latn", "name": "Finnish", "native": "Suomi", "iso": "fi"},
    {"code": "dan_Latn", "name": "Danish", "native": "Dansk", "iso": "da"},
    {"code": "nor_Latn", "name": "Norwegian", "native": "Norsk", "iso": "no"},
    {"code": "heb_Hebr", "name": "Hebrew", "native": "Hebrew", "iso": "he"},
]

FLORES200_FALLBACK = """
Acehnese (Arabic script)|ace_Arab
Acehnese (Latin script)|ace_Latn
Mesopotamian Arabic|acm_Arab
Taizzi-Adeni Arabic|acq_Arab
Tunisian Arabic|aeb_Arab
Afrikaans|afr_Latn
South Levantine Arabic|ajp_Arab
Akan|aka_Latn
Amharic|amh_Ethi
North Levantine Arabic|apc_Arab
Modern Standard Arabic|arb_Arab
Modern Standard Arabic (Romanized)|arb_Latn
Najdi Arabic|ars_Arab
Moroccan Arabic|ary_Arab
Egyptian Arabic|arz_Arab
Assamese|asm_Beng
Asturian|ast_Latn
Awadhi|awa_Deva
Central Aymara|ayr_Latn
South Azerbaijani|azb_Arab
North Azerbaijani|azj_Latn
Bashkir|bak_Cyrl
Bambara|bam_Latn
Balinese|ban_Latn
Belarusian|bel_Cyrl
Bemba|bem_Latn
Bengali|ben_Beng
Bhojpuri|bho_Deva
Banjar (Arabic script)|bjn_Arab
Banjar (Latin script)|bjn_Latn
Standard Tibetan|bod_Tibt
Bosnian|bos_Latn
Buginese|bug_Latn
Bulgarian|bul_Cyrl
Catalan|cat_Latn
Cebuano|ceb_Latn
Czech|ces_Latn
Chokwe|cjk_Latn
Central Kurdish|ckb_Arab
Crimean Tatar|crh_Latn
Welsh|cym_Latn
Danish|dan_Latn
German|deu_Latn
Southwestern Dinka|dik_Latn
Dyula|dyu_Latn
Dzongkha|dzo_Tibt
Greek|ell_Grek
English|eng_Latn
Esperanto|epo_Latn
Estonian|est_Latn
Basque|eus_Latn
Ewe|ewe_Latn
Faroese|fao_Latn
Fijian|fij_Latn
Finnish|fin_Latn
Fon|fon_Latn
French|fra_Latn
Friulian|fur_Latn
Nigerian Fulfulde|fuv_Latn
Scottish Gaelic|gla_Latn
Irish|gle_Latn
Galician|glg_Latn
Guarani|grn_Latn
Gujarati|guj_Gujr
Haitian Creole|hat_Latn
Hausa|hau_Latn
Hebrew|heb_Hebr
Hindi|hin_Deva
Chhattisgarhi|hne_Deva
Croatian|hrv_Latn
Hungarian|hun_Latn
Armenian|hye_Armn
Igbo|ibo_Latn
Ilocano|ilo_Latn
Indonesian|ind_Latn
Icelandic|isl_Latn
Italian|ita_Latn
Javanese|jav_Latn
Japanese|jpn_Jpan
Kabyle|kab_Latn
Jingpho|kac_Latn
Kamba|kam_Latn
Kannada|kan_Knda
Kashmiri (Arabic script)|kas_Arab
Kashmiri (Devanagari script)|kas_Deva
Georgian|kat_Geor
Central Kanuri (Arabic script)|knc_Arab
Central Kanuri (Latin script)|knc_Latn
Kazakh|kaz_Cyrl
Kabiye|kbp_Latn
Kabuverdianu|kea_Latn
Khmer|khm_Khmr
Kikuyu|kik_Latn
Kinyarwanda|kin_Latn
Kyrgyz|kir_Cyrl
Kimbundu|kmb_Latn
Northern Kurdish|kmr_Latn
Kikongo|kon_Latn
Korean|kor_Hang
Lao|lao_Laoo
Ligurian|lij_Latn
Limburgish|lim_Latn
Lingala|lin_Latn
Lithuanian|lit_Latn
Lombard|lmo_Latn
Latgalian|ltg_Latn
Luxembourgish|ltz_Latn
Luba-Kasai|lua_Latn
Ganda|lug_Latn
Luo|luo_Latn
Mizo|lus_Latn
Standard Latvian|lvs_Latn
Magahi|mag_Deva
Maithili|mai_Deva
Malayalam|mal_Mlym
Marathi|mar_Deva
Minangkabau (Arabic script)|min_Arab
Minangkabau (Latin script)|min_Latn
Macedonian|mkd_Cyrl
Plateau Malagasy|plt_Latn
Maltese|mlt_Latn
Meitei (Bengali script)|mni_Beng
Halh Mongolian|khk_Cyrl
Mossi|mos_Latn
Maori|mri_Latn
Burmese|mya_Mymr
Dutch|nld_Latn
Norwegian Nynorsk|nno_Latn
Norwegian Bokmal|nob_Latn
Nepali|npi_Deva
Northern Sotho|nso_Latn
Nuer|nus_Latn
Nyanja|nya_Latn
Occitan|oci_Latn
West Central Oromo|gaz_Latn
Odia|ory_Orya
Pangasinan|pag_Latn
Eastern Panjabi|pan_Guru
Papiamento|pap_Latn
Western Persian|pes_Arab
Polish|pol_Latn
Portuguese|por_Latn
Dari|prs_Arab
Southern Pashto|pbt_Arab
Ayacucho Quechua|quy_Latn
Romanian|ron_Latn
Rundi|run_Latn
Russian|rus_Cyrl
Sango|sag_Latn
Sanskrit|san_Deva
Santali|sat_Olck
Sicilian|scn_Latn
Shan|shn_Mymr
Sinhala|sin_Sinh
Slovak|slk_Latn
Slovenian|slv_Latn
Samoan|smo_Latn
Shona|sna_Latn
Sindhi|snd_Arab
Somali|som_Latn
Southern Sotho|sot_Latn
Spanish|spa_Latn
Tosk Albanian|als_Latn
Sardinian|srd_Latn
Serbian|srp_Cyrl
Swati|ssw_Latn
Sundanese|sun_Latn
Swedish|swe_Latn
Swahili|swh_Latn
Silesian|szl_Latn
Tamil|tam_Taml
Tatar|tat_Cyrl
Telugu|tel_Telu
Tajik|tgk_Cyrl
Tagalog|tgl_Latn
Thai|tha_Thai
Tigrinya|tir_Ethi
Tamasheq (Latin script)|taq_Latn
Tamasheq (Tifinagh script)|taq_Tfng
Tok Pisin|tpi_Latn
Tswana|tsn_Latn
Tsonga|tso_Latn
Turkmen|tuk_Latn
Tumbuka|tum_Latn
Turkish|tur_Latn
Twi|twi_Latn
Central Atlas Tamazight|tzm_Tfng
Uyghur|uig_Arab
Ukrainian|ukr_Cyrl
Umbundu|umb_Latn
Urdu|urd_Arab
Northern Uzbek|uzn_Latn
Venetian|vec_Latn
Vietnamese|vie_Latn
Waray|war_Latn
Wolof|wol_Latn
Xhosa|xho_Latn
Eastern Yiddish|ydd_Hebr
Yoruba|yor_Latn
Yue Chinese|yue_Hant
Chinese (Simplified)|zho_Hans
Chinese (Traditional)|zho_Hant
Standard Malay|zsm_Latn
Zulu|zul_Latn
"""

ISO_TO_NLLB = {item["iso"]: item["code"] for item in LANGUAGES if "iso" in item}

KNOWN_CODES = {item["code"] for item in LANGUAGES}
for line in FLORES200_FALLBACK.strip().splitlines():
    name, code = line.split("|", 1)
    if code not in KNOWN_CODES:
        LANGUAGES.append({"code": code, "name": name, "native": name})
        KNOWN_CODES.add(code)

for code in FAIRSEQ_LANGUAGE_CODES:
    if code not in KNOWN_CODES:
        LANGUAGES.append({"code": code, "name": code.replace("_", " "), "native": code})
        KNOWN_CODES.add(code)

DEFAULT_CONFIG = {
    "host": "127.0.0.1",
    "port": 8765,
    "default_source_language": "auto",
    "default_target_language": "eng_Latn",
    "default_mode": "fast",
    "theme": "system",
    "models": {
        "fast": "facebook/nllb-200-1.3B",
        "accurate": "facebook/nllb-200-3.3B",
    },
    "generation": {"max_length": 512, "num_beams": 4},
}


def load_config() -> dict[str, Any]:
    if not CONFIG_PATH.exists() or CONFIG_PATH.stat().st_size == 0:
        return DEFAULT_CONFIG.copy()
    with CONFIG_PATH.open("r", encoding="utf-8") as handle:
        loaded = json.load(handle)
    config = DEFAULT_CONFIG.copy()
    config.update(loaded)
    config["models"] = DEFAULT_CONFIG["models"] | loaded.get("models", {})
    config["generation"] = DEFAULT_CONFIG["generation"] | loaded.get("generation", {})
    return config


class TranslationEngine:
    def __init__(self, config: dict[str, Any]) -> None:
        self.config = config
        self._torch = None
        self._transformers: tuple[Any, Any] | None = None
        self.device = "unavailable"
        self.dtype = None
        self._pipelines: dict[str, tuple[Any, Any]] = {}
        self._lock = threading.Lock()
        self._prepare_runtime(raise_on_missing=False)

    def status(self) -> dict[str, Any]:
        self._prepare_runtime(raise_on_missing=False)
        return {
            "device": self.device,
            "cuda": bool(self._torch and self._torch.cuda.is_available()),
            "loaded_modes": sorted(self._pipelines),
            "models": self.config["models"],
            "ready": bool(self._torch and self._transformers),
        }

    def _prepare_runtime(self, raise_on_missing: bool) -> bool:
        if self._torch and self._transformers:
            return True
        try:
            import torch
            from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
        except Exception as exc:
            self.device = "unavailable"
            if raise_on_missing:
                raise RuntimeError(
                    "Missing AI dependencies. Run: pip install -r requirements.txt"
                ) from exc
            return False

        self._torch = torch
        self._transformers = (AutoTokenizer, AutoModelForSeq2SeqLM)
        self.device = "cuda" if torch.cuda.is_available() else "cpu"
        self.dtype = torch.float16 if self.device == "cuda" else torch.float32
        return True

    def _load(self, mode: str) -> tuple[Any, Any]:
        self._prepare_runtime(raise_on_missing=True)
        AutoTokenizer, AutoModelForSeq2SeqLM = self._transformers
        if mode not in self.config["models"]:
            mode = "fast"
        with self._lock:
            if mode in self._pipelines:
                return self._pipelines[mode]
            model_name = self.config["models"][mode]
            tokenizer = AutoTokenizer.from_pretrained(model_name)
            model = AutoModelForSeq2SeqLM.from_pretrained(model_name, torch_dtype=self.dtype)
            model.to(self.device)
            model.eval()
            self._pipelines[mode] = (tokenizer, model)
            return tokenizer, model

    def translate(self, text: str, source: str, target: str, mode: str) -> dict[str, Any]:
        text = text.strip()
        if not text:
            raise ValueError("Text is empty.")
        if target == "auto":
            raise ValueError("Target language cannot be automatic.")

        detected = None
        if source == "auto":
            source, detected = self.detect_language(text)

        tokenizer, model = self._load(mode)
        tokenizer.src_lang = source
        encoded = tokenizer(text, return_tensors="pt", truncation=True, max_length=512).to(self.device)
        forced_bos_token_id = tokenizer.convert_tokens_to_ids(target)
        with self._torch.inference_mode():
            generated = model.generate(
                **encoded,
                forced_bos_token_id=forced_bos_token_id,
                max_length=int(self.config["generation"]["max_length"]),
                num_beams=int(self.config["generation"]["num_beams"]),
            )
        return {
            "translation": tokenizer.batch_decode(generated, skip_special_tokens=True)[0],
            "source_language": source,
            "target_language": target,
            "detected_language": detected,
            "mode": mode,
            "device": self.device,
        }

    @staticmethod
    def detect_language(text: str) -> tuple[str, dict[str, str] | None]:
        if detect is None:
            return "eng_Latn", {"iso": "en", "code": "eng_Latn"}
        try:
            iso = detect(text)
        except LangDetectException:
            iso = "en"
        code = ISO_TO_NLLB.get(iso, "eng_Latn")
        return code, {"iso": iso, "code": code}


def create_app(config: dict[str, Any] | None = None) -> Flask:
    config = config or load_config()
    app = Flask(__name__, static_folder=str(FRONTEND_DIR), static_url_path="")
    CORS(app)
    engine = TranslationEngine(config)

    @app.get("/")
    def index():
        return send_from_directory(FRONTEND_DIR, "index.html")

    @app.get("/api/config")
    def get_config():
        public_config = {key: value for key, value in config.items() if key != "models"}
        return jsonify(public_config)

    @app.get("/api/languages")
    def languages():
        return jsonify({"languages": LANGUAGES})

    @app.get("/api/status")
    def status():
        return jsonify(engine.status())

    @app.post("/api/translate")
    def translate():
        payload = request.get_json(silent=True) or {}
        try:
            result = engine.translate(
                text=str(payload.get("text", "")),
                source=str(payload.get("source_language", config["default_source_language"])),
                target=str(payload.get("target_language", config["default_target_language"])),
                mode=str(payload.get("mode", config["default_mode"])),
            )
            return jsonify(result)
        except Exception as exc:
            return jsonify({"error": str(exc)}), 400

    @app.post("/api/detect")
    def detect_endpoint():
        payload = request.get_json(silent=True) or {}
        code, detected = engine.detect_language(str(payload.get("text", "")))
        return jsonify({"language": code, "detected": detected})

    return app
