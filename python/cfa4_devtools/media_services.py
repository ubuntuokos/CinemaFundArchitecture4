"""Optional cloud media services for CFA4 helper tools: transcription/subtitles and text-to-speech.

These are third-party Apify Store Actors selected from cporter202/API-mega-list. They are not part of the
FilmCraft product (which stays pure Rust and offline) and nothing here is linked into it. Selection criteria,
licence notes and excluded candidates are documented in ``docs/media-services.md``.

Standard library only. Every call sends your media URL or text to a third party: use it only for material you
have the rights to process.

CLI::

    APIFY_TOKEN=... python3 -m cfa4_devtools.media_services list
    APIFY_TOKEN=... python3 -m cfa4_devtools.media_services transcribe https://example.com/clip.mp4
    APIFY_TOKEN=... OPENAI_API_KEY=... python3 -m cfa4_devtools.media_services tts "Hello" --forward-openai-key
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping
from dataclasses import asdict, dataclass, field
from typing import Any

APIFY_API = "https://api.apify.com/v2"
SOURCE_LIST = "https://github.com/cporter202/API-mega-list"
SOURCE_COMMIT = "b7c914f5abd7d0f7562bd1f2036411d3c8f7c29e"

MAX_RESPONSE_BYTES = 32 * 1024 * 1024
MAX_TTS_CHARS = 100_000
MAX_URL_CHARS = 8192
DEFAULT_TIMEOUT_S = 300
TTS_VOICES = ("alloy", "echo", "fable", "onyx", "nova", "shimmer")
TTS_FORMATS = ("mp3", "wav", "opus", "aac")


class MediaServiceError(Exception):
    """Any failure: bad input, missing credentials, network/HTTP error or an unusable response."""


@dataclass(frozen=True)
class Service:
    id: str
    kind: str  # "transcribe" | "tts"
    actor: str  # Apify actor id, "username~actor-name"
    store_url: str
    engine: str
    needs_openai_key: bool
    notes: str


SERVICES: dict[str, Service] = {
    s.id: s
    for s in (
        Service(
            id="deepgram-stt",
            kind="transcribe",
            actor="hgservices~speech-to-text",
            store_url="https://apify.com/hgservices/speech-to-text",
            engine="Deepgram Nova-3 (Whisper large fallback)",
            needs_openai_key=False,
            notes="Speaker diarization, smart formatting; plain transcript text, no subtitle files.",
        ),
        Service(
            id="whisper-subtitles",
            kind="transcribe",
            actor="dami_studio~video-audio-transcriber",
            store_url="https://apify.com/dami_studio/video-audio-transcriber",
            engine="OpenAI whisper-1 (your own OpenAI key)",
            needs_openai_key=True,
            notes="Word- and segment-level timestamps; SRT/VTT/TXT subtitle files.",
        ),
        Service(
            id="openai-tts",
            kind="tts",
            actor="dami_studio~ai-tts-voiceover",
            store_url="https://apify.com/dami_studio/ai-tts-voiceover",
            engine="OpenAI TTS tts-1 / tts-1-hd (your own OpenAI key)",
            needs_openai_key=True,
            notes="Voiceover audio file (mp3/wav/opus/aac); disclose to listeners that the voice is AI-generated.",
        ),
    )
}


@dataclass
class Transcript:
    service: str
    text: str
    language: str | None = None
    duration_seconds: float | None = None
    speaker_count: int | None = None
    segments: list[dict[str, Any]] = field(default_factory=list)
    words: list[dict[str, Any]] = field(default_factory=list)
    subtitle_urls: dict[str, str] = field(default_factory=dict)


@dataclass
class Voiceover:
    service: str
    audio_url: str
    duration_seconds: float | None = None
    characters: int | None = None


# (url, body, headers, timeout_s) -> (status, response bytes). Injectable for tests.
Transport = Callable[[str, bytes, Mapping[str, str], float], tuple[int, bytes]]


def urllib_transport(url: str, body: bytes, headers: Mapping[str, str], timeout_s: float) -> tuple[int, bytes]:
    request = urllib.request.Request(url, data=body, headers=dict(headers), method="POST")
    try:
        with urllib.request.urlopen(request, timeout=timeout_s) as response:
            return response.status, response.read(MAX_RESPONSE_BYTES + 1)
    except urllib.error.HTTPError as error:
        return error.code, error.read(64 * 1024)
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise MediaServiceError(f"network error: {error}") from error


class MediaServices:
    def __init__(self, apify_token: str, transport: Transport = urllib_transport, timeout_s: float = DEFAULT_TIMEOUT_S):
        if not apify_token.strip():
            raise MediaServiceError("missing Apify token (set APIFY_TOKEN)")
        if not 0 < timeout_s <= 3600:
            raise MediaServiceError("timeout must be between 0 and 3600 seconds")
        self._token = apify_token.strip()
        self._transport = transport
        self._timeout_s = timeout_s

    def run_actor(self, actor: str, actor_input: Mapping[str, Any]) -> list[dict[str, Any]]:
        """Run an actor synchronously and return its dataset items."""
        query = urllib.parse.urlencode({"timeout": int(min(self._timeout_s, DEFAULT_TIMEOUT_S))})
        url = f"{APIFY_API}/acts/{urllib.parse.quote(actor, safe='~')}/run-sync-get-dataset-items?{query}"
        headers = {"Authorization": f"Bearer {self._token}", "Content-Type": "application/json"}
        body = json.dumps(dict(actor_input)).encode("utf-8")
        status, payload = self._transport(url, body, headers, self._timeout_s + 30)
        if len(payload) > MAX_RESPONSE_BYTES:
            raise MediaServiceError("response too large")
        if status == 408:
            raise MediaServiceError(f"{actor}: run did not finish within {self._timeout_s:.0f} s")
        if not 200 <= status < 300:
            raise MediaServiceError(f"{actor}: HTTP {status}: {_error_message(payload)}")
        try:
            items = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise MediaServiceError(f"{actor}: response is not JSON") from error
        if not isinstance(items, list):
            raise MediaServiceError(f"{actor}: expected a list of dataset items")
        return [item for item in items if isinstance(item, dict)]

    def transcribe(
        self,
        media_url: str,
        service: str = "deepgram-stt",
        language: str = "auto",
        openai_api_key: str | None = None,
        forward_openai_key: bool = False,
    ) -> Transcript:
        svc = _service(service, "transcribe")
        _check_url(media_url)
        if svc.id == "deepgram-stt":
            items = self.run_actor(svc.actor, {"mediaUrl": media_url, "diarize": True, "smartFormat": True})
            row = _first_ok(svc, items)
            text = _str(row.get("transcript"))
            if not text:
                raise MediaServiceError(f"{svc.id}: {_str(row.get('statusMessage')) or 'empty transcript'}")
            return Transcript(
                service=svc.id,
                text=text,
                language=_str(row.get("language")) or None,
                duration_seconds=_num(row.get("durationSeconds")),
                speaker_count=_int(row.get("speakerCount")),
            )
        key = _openai_key(svc, openai_api_key, forward_openai_key)
        actor_input: dict[str, Any] = {
            "mediaUrl": media_url,
            "language": language or "auto",
            "wordTimestamps": True,
            "outputFormats": ["srt", "vtt", "txt"],
            "openaiApiKey": key,
        }
        row = _first_ok(svc, self.run_actor(svc.actor, actor_input))
        subtitle_urls = {fmt: url for fmt in ("srt", "vtt", "txt") if (url := _str(row.get(f"{fmt}Url")))}
        return Transcript(
            service=svc.id,
            text=_str(row.get("text")),
            language=_str(row.get("language")) or None,
            duration_seconds=_num(row.get("durationSeconds")),
            segments=_dicts(row.get("segments")),
            words=_dicts(row.get("words")),
            subtitle_urls=subtitle_urls,
        )

    def synthesize(
        self,
        text: str,
        voice: str = "onyx",
        audio_format: str = "mp3",
        speed: float = 1.0,
        hd: bool = False,
        service: str = "openai-tts",
        openai_api_key: str | None = None,
        forward_openai_key: bool = False,
    ) -> Voiceover:
        svc = _service(service, "tts")
        if not text.strip():
            raise MediaServiceError("text is empty")
        if len(text) > MAX_TTS_CHARS:
            raise MediaServiceError(f"text longer than {MAX_TTS_CHARS} characters")
        if voice not in TTS_VOICES:
            raise MediaServiceError(f"unknown voice {voice!r}; choose one of {', '.join(TTS_VOICES)}")
        if audio_format not in TTS_FORMATS:
            raise MediaServiceError(f"unknown format {audio_format!r}; choose one of {', '.join(TTS_FORMATS)}")
        if not 0.25 <= speed <= 4.0:
            raise MediaServiceError("speed must be between 0.25 and 4.0")
        key = _openai_key(svc, openai_api_key, forward_openai_key)
        actor_input = {
            "text": text,
            "voice": voice,
            "model": "tts-1-hd" if hd else "tts-1",
            "format": audio_format,
            "speed": speed,
            "openaiApiKey": key,
        }
        row = _first_ok(svc, self.run_actor(svc.actor, actor_input))
        audio_url = _str(row.get("audioUrl"))
        if not audio_url:
            raise MediaServiceError(f"{svc.id}: no audio produced")
        return Voiceover(
            service=svc.id,
            audio_url=audio_url,
            duration_seconds=_num(row.get("durationSeconds")),
            characters=_int(row.get("characters")),
        )


def _service(service_id: str, kind: str) -> Service:
    svc = SERVICES.get(service_id)
    if svc is None or svc.kind != kind:
        names = ", ".join(s.id for s in SERVICES.values() if s.kind == kind)
        raise MediaServiceError(f"unknown {kind} service {service_id!r}; choose one of {names}")
    return svc


def _openai_key(svc: Service, key: str | None, forward: bool) -> str:
    if not forward:
        raise MediaServiceError(
            f"{svc.id} runs your OpenAI key through a third-party actor ({svc.store_url}); "
            "pass forward_openai_key=True / --forward-openai-key to allow it"
        )
    if not key or not key.strip():
        raise MediaServiceError(f"{svc.id} needs an OpenAI API key (set OPENAI_API_KEY)")
    return key.strip()


def _check_url(url: str) -> None:
    if len(url) > MAX_URL_CHARS:
        raise MediaServiceError("media URL too long")
    try:
        parsed = urllib.parse.urlparse(url)
    except ValueError as error:
        raise MediaServiceError(f"invalid media URL: {error}") from error
    if parsed.scheme not in ("http", "https") or not parsed.netloc:
        raise MediaServiceError("media URL must be a public http(s) link to the media file")


def _first_ok(svc: Service, items: list[dict[str, Any]]) -> dict[str, Any]:
    for item in items:
        if item.get("ok") is False or item.get("error"):
            raise MediaServiceError(f"{svc.id}: {_str(item.get('error')) or 'run failed'}")
        return item
    raise MediaServiceError(f"{svc.id}: actor returned no results")


def _error_message(payload: bytes) -> str:
    try:
        data = json.loads(payload.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return payload[:200].decode("utf-8", errors="replace")
    if isinstance(data, dict) and isinstance(data.get("error"), dict):
        return _str(data["error"].get("message")) or "unknown error"
    return "unknown error"


def _str(value: Any) -> str:
    return value if isinstance(value, str) else ""


def _num(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    return number if number == number and abs(number) != float("inf") and number >= 0 else None


def _int(value: Any) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else None


def _dicts(value: Any) -> list[dict[str, Any]]:
    return [item for item in value if isinstance(item, dict)] if isinstance(value, list) else []


def main(argv: list[str] | None = None, transport: Transport = urllib_transport) -> int:
    parser = argparse.ArgumentParser(prog="cfa4_devtools.media_services", description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("list", help="list the selected services")
    tr = sub.add_parser("transcribe", help="transcribe a public audio/video file URL")
    tr.add_argument("media_url")
    tr.add_argument("--service", default="deepgram-stt", choices=[s.id for s in SERVICES.values() if s.kind == "transcribe"])
    tr.add_argument("--language", default="auto")
    tr.add_argument("--forward-openai-key", action="store_true")
    tts = sub.add_parser("tts", help="synthesise a voiceover from text ('-' reads stdin)")
    tts.add_argument("text")
    tts.add_argument("--voice", default="onyx", choices=TTS_VOICES)
    tts.add_argument("--format", dest="audio_format", default="mp3", choices=TTS_FORMATS)
    tts.add_argument("--speed", type=float, default=1.0)
    tts.add_argument("--hd", action="store_true")
    tts.add_argument("--forward-openai-key", action="store_true")
    tts.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_S)
    tr.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_S)
    args = parser.parse_args(argv)

    if args.command == "list":
        for svc in SERVICES.values():
            print(f"{svc.id:18} {svc.kind:10} {svc.engine}  {svc.store_url}")
        return 0
    try:
        client = MediaServices(os.environ.get("APIFY_TOKEN", ""), transport, args.timeout)
        openai_key = os.environ.get("OPENAI_API_KEY")
        if args.command == "transcribe":
            result: Transcript | Voiceover = client.transcribe(
                args.media_url, args.service, args.language, openai_key, args.forward_openai_key
            )
        else:
            text = sys.stdin.read(MAX_TTS_CHARS + 1) if args.text == "-" else args.text
            result = client.synthesize(
                text, args.voice, args.audio_format, args.speed, args.hd, "openai-tts", openai_key, args.forward_openai_key
            )
    except MediaServiceError as error:
        print(f"media_services: {error}", file=sys.stderr)
        return 1
    print(json.dumps(asdict(result), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
