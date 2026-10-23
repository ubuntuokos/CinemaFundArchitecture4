# Optional cloud media services (helper tools)

`python/cfa4_devtools/media_services.py` gives CFA4 helper tools and future feature prototypes access to a
few third-party cloud services: **speech-to-text / subtitles** and **text-to-speech**. It is developer
tooling only. The FilmCraft product stays pure Rust and offline (see [`CLAUDE.md`](../CLAUDE.md)); nothing
here is linked into a crate, and no generated media is committed (generated audio that ever lands in the
repository is an asset under [`AGENTS.md`](../AGENTS.md) §1 and needs an attribution sidecar).

```sh
export APIFY_TOKEN=...                                   # your Apify account token
PYTHONPATH=python python3 -m cfa4_devtools.media_services list
PYTHONPATH=python python3 -m cfa4_devtools.media_services transcribe https://example.com/interview.mp4
export OPENAI_API_KEY=...                                # only for the OpenAI-backed services
PYTHONPATH=python python3 -m cfa4_devtools.media_services transcribe https://example.com/a.mp3 \
    --service whisper-subtitles --language hu --forward-openai-key
PYTHONPATH=python python3 -m cfa4_devtools.media_services tts "Szöveg" --voice nova --forward-openai-key
```

From Python: `MediaServices(token).transcribe(url)` returns a `Transcript`, `synthesize(text, ...)` a
`Voiceover`; every failure is a `MediaServiceError`. Tests: `PYTHONPATH=python python3 -m unittest
discover -s python/tests -p 'test_media_services.py'` (offline, fake transport).

## Selected services

Source: [cporter202/API-mega-list](https://github.com/cporter202/API-mega-list) at commit
`b7c914f5abd7d0f7562bd1f2036411d3c8f7c29e`. All entries there are Apify Store community Actors.

| id | Apify Actor | Engine | What you get |
|---|---|---|---|
| `deepgram-stt` | [hgservices/speech-to-text](https://apify.com/hgservices/speech-to-text) | Deepgram Nova-3 (Whisper large fallback) | Transcript text with speaker labels; no own key needed |
| `whisper-subtitles` | [dami_studio/video-audio-transcriber](https://apify.com/dami_studio/video-audio-transcriber) | OpenAI `whisper-1` (your key) | Segment and word timestamps, SRT/VTT/TXT files |
| `openai-tts` | [dami_studio/ai-tts-voiceover](https://apify.com/dami_studio/ai-tts-voiceover) | OpenAI `tts-1` / `tts-1-hd` (your key) | Voiceover file (mp3/wav/opus/aac) |

Inputs must be public, direct links to the media file. Pricing is per use and set by each Actor (checked
2026-10-23); the OpenAI-backed ones also bill your OpenAI key.

## Licence and terms basis (checked 2026-10-23; not legal advice)

- **No Actor has an open-source licence or public source.** Under Apify's
  [Actor Terms](https://docs.apify.com/legal/actor-terms-and-conditions) the creator keeps all rights in
  the Actor (§3.1) and you may run it under Apify's terms and any creator terms (§2.2; none are published
  for these three). Nothing from the Actors is copied into this repository: the module only calls their
  public API.
- **Outputs belong to you:** Actor Terms §3.2 leaves the IP in Actor inputs and outputs with the user.
- **Engine terms allow commercial use of the output:** OpenAI's terms assign output to the customer
  (OpenAI's usage policies require disclosing AI-generated voices to listeners); Deepgram's
  [terms](https://deepgram.com/terms) §3.4 leave output with the customer but forbid using it to build
  competing models (§2.4) and allow Deepgram to train on submitted audio unless opted out (§3.2–3.3).
- Selection rule: an Actor qualified only if it names its engine **and** that engine's terms allow
  commercial use of the output. Unknown engines were rejected because their licence cannot be checked.

## Privacy and security

- Media URLs and text are sent to Apify, the Actor's creator and the engine provider. Process only
  material you have the rights to, and nothing confidential.
- The `whisper-subtitles` and `openai-tts` Actors receive **your OpenAI API key as Actor input**, which the
  Actor's creator can access (Actor Terms §4.2). The module refuses unless you pass
  `forward_openai_key=True` / `--forward-openai-key`. Use a separate, spend-limited key.
- The Apify token is sent in the `Authorization` header, never in the URL.

## Considered and rejected

- **Background removal:** none qualified. [ainz/background-remover](https://apify.com/ainz/background-remover)
  uses BRIA RMBG, licensed [CC BY-NC 4.0](https://huggingface.co/briaai/RMBG-2.0) (non-commercial);
  seemuapps/image-background-remover, akash9078/ai-bg-remover and stefanie-rink/image-background-remover
  do not name their model.
- **Text-to-speech:** saswave/ai-text-to-speech is deprecated, names no engine and has no Hungarian;
  marielise.dev/pdf-to-mp3 relies on the unofficial Microsoft Edge read-aloud service; the others name no
  engine.
- **Speech-to-text with unnamed engines:** sian.agency/incredibly-fast-audio-transcriber,
  parseforge/audio-transcriber, tictechid/vanzi-universal-transcriber, the Hugging Face audio Actor
  (unnamed default model). marielise.dev/video-transcriber-ultimate (Groq Whisper) qualifies on terms but
  costs over $1 per file and fails above ~50 minutes, so it adds nothing over the two selected.
- **Caption scrapers** (most "transcript" entries, e.g. YouTube/TikTok/TED transcript scrapers) only fetch
  captions that a platform already published; they cannot process your own footage.
- **Audio noise removal** (parseforge/noise-remover): engine not named.
