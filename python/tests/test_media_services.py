"""Standard-library tests for the optional cloud media services (no network access)."""

from __future__ import annotations

import io
import json
import unittest
from collections.abc import Mapping
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

from cfa4_devtools import media_services as ms


class FakeTransport:
    def __init__(self, status: int = 200, payload: object = (), raw: bytes | None = None) -> None:
        self.status = status
        self.raw = raw if raw is not None else json.dumps(payload).encode("utf-8")
        self.calls: list[tuple[str, dict, dict]] = []

    def __call__(self, url: str, body: bytes, headers: Mapping[str, str], timeout_s: float) -> tuple[int, bytes]:
        self.calls.append((url, json.loads(body), dict(headers)))
        return self.status, self.raw


def client(transport: FakeTransport) -> ms.MediaServices:
    return ms.MediaServices("token-123", transport=transport, timeout_s=60)


class RegistryTests(unittest.TestCase):
    def test_selected_services(self) -> None:
        self.assertEqual(set(ms.SERVICES), {"deepgram-stt", "whisper-subtitles", "openai-tts"})
        for svc in ms.SERVICES.values():
            self.assertIn("~", svc.actor)
            self.assertTrue(svc.store_url.startswith("https://apify.com/"))


class TranscribeTests(unittest.TestCase):
    def test_deepgram_request_and_result(self) -> None:
        t = FakeTransport(payload=[{"transcript": "[Speaker 0] Hello", "language": "en", "durationSeconds": 12.5, "speakerCount": 1}])
        result = client(t).transcribe("https://example.com/a.mp4")
        url, body, headers = t.calls[0]
        self.assertIn("/acts/hgservices~speech-to-text/run-sync-get-dataset-items", url)
        self.assertNotIn("token-123", url)
        self.assertEqual(headers["Authorization"], "Bearer token-123")
        self.assertEqual(body, {"mediaUrl": "https://example.com/a.mp4", "diarize": True, "smartFormat": True})
        self.assertEqual(result.text, "[Speaker 0] Hello")
        self.assertEqual(result.speaker_count, 1)
        self.assertEqual(result.duration_seconds, 12.5)

    def test_whisper_requires_explicit_key_forwarding(self) -> None:
        t = FakeTransport(payload=[{}])
        with self.assertRaises(ms.MediaServiceError):
            client(t).transcribe("https://example.com/a.mp3", "whisper-subtitles", openai_api_key="sk-x")
        with self.assertRaises(ms.MediaServiceError):
            client(t).transcribe("https://example.com/a.mp3", "whisper-subtitles", forward_openai_key=True)
        self.assertEqual(t.calls, [])

    def test_whisper_subtitles(self) -> None:
        row = {
            "ok": True, "text": "Hi there", "language": "en", "durationSeconds": 3,
            "segments": [{"start": 0, "end": 3, "text": "Hi there"}, "junk"],
            "words": [{"word": "Hi", "start": 0, "end": 0.4}],
            "srtUrl": "https://x/a.srt", "vttUrl": "https://x/a.vtt", "txtUrl": 5,
        }
        t = FakeTransport(payload=[row])
        result = client(t).transcribe("https://example.com/a.mp3", "whisper-subtitles", "hu", "sk-x", True)
        self.assertEqual(t.calls[0][1]["openaiApiKey"], "sk-x")
        self.assertEqual(t.calls[0][1]["language"], "hu")
        self.assertEqual(result.subtitle_urls, {"srt": "https://x/a.srt", "vtt": "https://x/a.vtt"})
        self.assertEqual(len(result.segments), 1)
        self.assertEqual(result.words[0]["word"], "Hi")

    def test_rejects_bad_urls(self) -> None:
        t = FakeTransport(payload=[])
        for url in ("", "file:///etc/passwd", "ftp://x/a.mp3", "https://", "http://[::1", "https://x/" + "a" * 9000):
            with self.assertRaises(ms.MediaServiceError, msg=url):
                client(t).transcribe(url)
        self.assertEqual(t.calls, [])

    def test_failed_row_is_an_error(self) -> None:
        t = FakeTransport(payload=[{"ok": False, "error": "unsupported file"}])
        with self.assertRaisesRegex(ms.MediaServiceError, "unsupported file"):
            client(t).transcribe("https://example.com/a.mp3", "whisper-subtitles", openai_api_key="k", forward_openai_key=True)

    def test_empty_deepgram_transcript_is_an_error(self) -> None:
        t = FakeTransport(payload=[{"transcript": "", "statusMessage": "no speech"}])
        with self.assertRaisesRegex(ms.MediaServiceError, "no speech"):
            client(t).transcribe("https://example.com/a.mp3")


class SynthesizeTests(unittest.TestCase):
    def test_tts_request_and_result(self) -> None:
        t = FakeTransport(payload=[{"audioUrl": "https://x/v.mp3", "durationSeconds": 2.1, "characters": 5}])
        result = client(t).synthesize("Hello", voice="nova", hd=True, openai_api_key="sk-x", forward_openai_key=True)
        body = t.calls[0][1]
        self.assertIn("/acts/dami_studio~ai-tts-voiceover/", t.calls[0][0])
        self.assertEqual((body["voice"], body["model"], body["format"]), ("nova", "tts-1-hd", "mp3"))
        self.assertEqual(result.audio_url, "https://x/v.mp3")

    def test_tts_validation(self) -> None:
        t = FakeTransport(payload=[])
        c = client(t)
        cases = [
            dict(text=" "), dict(text="x" * (ms.MAX_TTS_CHARS + 1)), dict(text="a", voice="bob"),
            dict(text="a", audio_format="flac"), dict(text="a", speed=0.0), dict(text="a", speed=float("nan")),
        ]
        for kwargs in cases:
            with self.assertRaises(ms.MediaServiceError, msg=str(kwargs)[:60]):
                c.synthesize(openai_api_key="k", forward_openai_key=True, **kwargs)
        self.assertEqual(t.calls, [])


class HostileResponseTests(unittest.TestCase):
    def check(self, transport: FakeTransport) -> None:
        with self.assertRaises(ms.MediaServiceError):
            client(transport).transcribe("https://example.com/a.mp3")

    def test_hostile_responses_never_escape_as_other_exceptions(self) -> None:
        self.check(FakeTransport(status=401, raw=b'{"error":{"message":"bad token"}}'))
        self.check(FakeTransport(status=500, raw=b"\xff\xfe garbage"))
        self.check(FakeTransport(status=408, raw=b""))
        self.check(FakeTransport(raw=b"not json"))
        self.check(FakeTransport(raw=b'{"a": 1}'))
        self.check(FakeTransport(payload=[]))
        self.check(FakeTransport(payload=[1, "x", None]))
        self.check(FakeTransport(raw=b"[" + b" " * ms.MAX_RESPONSE_BYTES + b"]"))

    def test_odd_field_types_are_dropped(self) -> None:
        t = FakeTransport(payload=[{"transcript": "ok", "durationSeconds": -1, "speakerCount": True, "language": 7}])
        result = client(t).transcribe("https://example.com/a.mp3")
        self.assertEqual((result.duration_seconds, result.speaker_count, result.language), (None, None, None))

    def test_constructor_validation(self) -> None:
        with self.assertRaises(ms.MediaServiceError):
            ms.MediaServices("  ")
        with self.assertRaises(ms.MediaServiceError):
            ms.MediaServices("t", timeout_s=0)


class CliTests(unittest.TestCase):
    def run_cli(self, argv: list[str], env: dict[str, str], transport: ms.Transport = ms.urllib_transport) -> tuple[int, str, str]:
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.dict("os.environ", env, clear=True), redirect_stdout(out), redirect_stderr(err):
            code = ms.main(argv, transport)
        return code, out.getvalue(), err.getvalue()

    def test_list(self) -> None:
        code, out, _ = self.run_cli(["list"], {})
        self.assertEqual(code, 0)
        self.assertIn("deepgram-stt", out)

    def test_missing_token_is_a_clean_error(self) -> None:
        code, _, err = self.run_cli(["transcribe", "https://example.com/a.mp3"], {})
        self.assertEqual(code, 1)
        self.assertIn("APIFY_TOKEN", err)

    def test_transcribe_through_cli(self) -> None:
        t = FakeTransport(payload=[{"transcript": "hello"}])
        code, out, _ = self.run_cli(["transcribe", "https://example.com/a.mp3"], {"APIFY_TOKEN": "t"}, t)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["text"], "hello")


if __name__ == "__main__":
    unittest.main()
