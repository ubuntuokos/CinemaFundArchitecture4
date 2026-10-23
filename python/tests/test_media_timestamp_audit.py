"""CFA4 F7 container metadata timing, synthetic-only regression fixtures."""
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from cfa4_devtools import media_timestamp_audit as audit


def good_payload():
    video = [
        {"stream_index": 0, "pts_time": f"{n / 25:.3f}",
         "dts_time": f"{n / 25:.3f}", "duration_time": "0.040"}
        for n in range(50)
    ]
    audio = [
        {"stream_index": 1, "pts_time": f"{n * 0.020:.3f}",
         "dts_time": f"{n * 0.020:.3f}", "duration_time": "0.020"}
        for n in range(100)
    ]
    return {
        "streams": [
            {"index": 0, "codec_type": "video", "codec_name": "h264", "time_base": "1/12800"},
            {"index": 1, "codec_type": "audio", "codec_name": "aac", "time_base": "1/48000"},
        ],
        "packets": video + audio,
    }


class TimestampAuditTests(unittest.TestCase):
    def test_video_audio_metadata_is_explicitly_limited(self):
        result = audit.timing_evidence(good_payload())
        self.assertEqual(result["status"], "PASS_CONTAINER_TIMESTAMPS_ONLY")
        self.assertIn("perceptual audio/video lip sync", result["not_proven"])
        self.assertEqual(result["streams"]["video"]["packets"], 50)

    def test_missing_audio_is_not_success(self):
        data = good_payload()
        data["streams"] = data["streams"][:1]
        with self.assertRaisesRegex(audit.MediaTimingError, "no audio stream"):
            audit.timing_evidence(data)

    def test_large_aac_offset_is_rejected(self):
        data = good_payload()
        for packet in data["packets"]:
            if packet["stream_index"] == 1:
                packet["pts_time"] = str(float(packet["pts_time"]) + 0.8)
                packet["dts_time"] = str(float(packet["dts_time"]) + 0.8)
        with self.assertRaisesRegex(audit.MediaTimingError, "skew exceeds threshold"):
            audit.timing_evidence(data)

    def test_nonmonotonic_decode_timestamps_fail(self):
        data = good_payload()
        data["packets"][4]["dts_time"] = "-1"
        with self.assertRaisesRegex(audit.MediaTimingError, "non-monotonic"):
            audit.timing_evidence(data)

    def test_packet_duration_cannot_be_zero(self):
        data = good_payload()
        data["packets"][0]["duration_time"] = "0"
        with self.assertRaisesRegex(audit.MediaTimingError, "nonpositive"):
            audit.timing_evidence(data)

    def test_nonfinite_packet_time_rejected(self):
        data = good_payload()
        data["packets"][0]["pts_time"] = "NaN"
        with self.assertRaisesRegex(audit.MediaTimingError, "non-finite"):
            audit.timing_evidence(data)

    def test_packet_limit_is_enforced(self):
        data = good_payload()
        data["packets"] = data["packets"] * 102
        with self.assertRaisesRegex(audit.MediaTimingError, "too many packets"):
            audit.timing_evidence(data)

    def test_missing_media_does_not_launch_ffprobe(self):
        with self.assertRaisesRegex(audit.MediaTimingError, "unavailable"):
            audit.ffprobe_payload(Path("/impossible/cfa4-missing.mp4"), "ffprobe", 10)

    def test_ffprobe_bounded_arguments(self):
        with tempfile.TemporaryDirectory() as folder:
            media = Path(folder) / "in.mp4"
            media.touch()
            with patch.object(audit.subprocess, "run", return_value=subprocess.CompletedProcess(
                args=[], returncode=0, stdout=b"{}", stderr=b"",
            )) as mocked:
                self.assertEqual(audit.ffprobe_payload(media, "ffprobe", 10), {})
                args = mocked.call_args.args[0]
                self.assertIn("0%+10", args)
                self.assertIn("-show_packets", args)

    def test_probe_window_over_limit_rejected(self):
        with self.assertRaisesRegex(audit.MediaTimingError, "must be 1"):
            audit.ffprobe_payload(Path("/missing"), "ffprobe", 90)


if __name__ == "__main__":
    unittest.main()
