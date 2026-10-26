"""Speech-to-text helper for the transcribe_audio tool.

Usage: python transcribe.py <audio url or file>
Prints {"language": ..., "text": ...} as JSON. Needs `pip install faster-whisper`;
the model (WHISPER_MODEL, default "base") is downloaded on first use into WHISPER_DIR.
"""
import json
import os
import sys
import tempfile
import urllib.request

from faster_whisper import WhisperModel

src = sys.argv[1]
path = src
if src.startswith(("http://", "https://")):
    fd, path = tempfile.mkstemp(suffix=".audio")
    with os.fdopen(fd, "wb") as f, urllib.request.urlopen(urllib.request.Request(src, headers={"User-Agent": "Mozilla/5.0"})) as r:
        f.write(r.read())

model = WhisperModel(os.environ.get("WHISPER_MODEL", "base"), device="cpu", compute_type="int8", download_root=os.environ.get("WHISPER_DIR"))
segments, info = model.transcribe(path, vad_filter=True)
text = " ".join(s.text.strip() for s in segments).strip()
if path != src:
    os.remove(path)
print(json.dumps({"language": info.language, "text": text}, ensure_ascii=False))
