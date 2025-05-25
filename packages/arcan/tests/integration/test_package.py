"""Integration tests for package installation and functionality."""

import subprocess
import sys

import pytest


@pytest.mark.integration
def test_package_import():
    """Test that the package can be imported."""
    import arcan

    assert hasattr(arcan, "__version__")
    assert hasattr(arcan, "__author__")
    assert hasattr(arcan, "__email__")


@pytest.mark.integration
def test_cli_entrypoint():
    """Test that the CLI entrypoint works."""
    result = subprocess.run(
        [sys.executable, "-m", "arcan", "--version"],
        capture_output=True,
        text=True,
        check=False,  # We're checking the return code manually
    )

    assert result.returncode == 0
    assert "Arcan version:" in result.stdout
