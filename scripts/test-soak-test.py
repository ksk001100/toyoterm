"""Regression checks for trend reporting and the bounded fixture."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("soak-test.py")
spec = importlib.util.spec_from_file_location("soak", SCRIPT)
soak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(soak)


class TrendTests(unittest.TestCase):
    def test_initial_cache_growth_is_excluded(self):
        rows = [{"elapsed_s": i * 60, "rss_bytes": (i if i < 5 else 5) * 1048576}
                for i in range(20)]
        metric = soak.summarize(rows, 300)["metrics"]["rss_bytes"]
        self.assertEqual(metric["slope_mib_per_min"], 0)

    def test_continuous_growth_is_visible(self):
        rows = [{"elapsed_s": i * 60, "rss_bytes": i * 2 * 1048576}
                for i in range(20)]
        metric = soak.summarize(rows, 300)["metrics"]["rss_bytes"]
        self.assertAlmostEqual(metric["slope_mib_per_min"], 2)
        self.assertGreater(metric["late_mean_mib"], metric["early_mean_mib"])

    def test_insufficient_samples_do_not_claim_stability(self):
        result = soak.summarize([{"elapsed_s": 1, "rss_bytes": 100}], 30)
        self.assertEqual(result["metrics"], {})
        self.assertIn("insufficient", result["status"])

    def test_growth_per_iteration_is_independent_of_speed(self):
        rows = [{"iteration": i * 100, "rss_bytes": i * 2 * 1048576}
                for i in range(20)]
        metric = soak.summarize(rows, 500, "iteration")["metrics"]["rss_bytes"]
        self.assertAlmostEqual(metric["slope_mib_per_100_iterations"], 2)

    def test_output_fixture_stops_at_line_limit(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "fixture", "--lines", "1010", "--delay", "0"],
                                capture_output=True, check=True, timeout=10)
        self.assertEqual(result.stdout.count(b"toyoterm soak output"), 1010)
        self.assertTrue(result.stdout.endswith(b"SOAK_FIXTURE_DONE\r\n"))

    def test_memory_sampler_observes_current_process(self):
        import os
        self.assertGreater(soak.memory(os.getpid())["rss_bytes"], 0)

    def test_runtime_errors_are_reported_even_when_workload_completed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "gui.log"
            path.write_text("WARN recovery error\nTRACE counters\n" + "ERROR resize failed\n" * 105,
                            encoding="utf-8")
            count, examples = soak.collect_log_errors(path)
            self.assertEqual(count, 105)
            self.assertEqual(len(examples), 100)

    def test_custom_config_requires_idle_before_creating_output(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            config = directory / "config.rb"
            config.write_text("Toyoterm.configure {}\n", encoding="utf-8")
            output = directory / "results"
            result = subprocess.run([sys.executable, str(SCRIPT), "gui", "--executable", "unused",
                                     "--output", str(output), "--config", str(config)],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn("--config requires --scenario idle", result.stderr)
            self.assertFalse(output.exists())

    def test_missing_custom_config_is_rejected_before_creating_output(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            output = directory / "results"
            result = subprocess.run([sys.executable, str(SCRIPT), "gui", "--executable", "unused",
                                     "--scenario", "idle", "--output", str(output),
                                     "--config", str(directory / "missing.rb")],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn("configuration is not a file", result.stderr)
            self.assertFalse(output.exists())

    def test_idle_config_path_and_hash_describe_selected_configuration(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            config = directory / "config.rb"
            config.write_text("Toyoterm.configure { |c| c.scrollback_lines = 42 }\n", encoding="utf-8")
            for name, extra_args, expected_config in (
                ("custom", ["--scenario", "idle", "--config", str(config)], config),
                ("default", [], soak.ROOT / "scripts/soak-config.rb"),
            ):
                with self.subTest(name=name):
                    output = directory / name
                    argv = [str(SCRIPT), "gui", "--executable", "unused", "--output", str(output), *extra_args]
                    with patch.object(sys, "argv", argv), patch.object(soak, "gui") as gui, \
                         patch.object(soak.subprocess, "check_output", return_value="test\n"):
                        soak.main()
                    self.assertEqual(gui.call_args.args[0].config, expected_config.resolve())
                    metadata = json.loads((output / "metadata.json").read_text(encoding="utf-8"))
                    self.assertEqual(metadata["config_path"], str(expected_config.resolve()))
                    self.assertEqual(metadata["config_sha256"], soak.sha256(expected_config))
                    self.assertEqual(metadata["fixture_sha256"], metadata["config_sha256"])


if __name__ == "__main__":
    unittest.main()
