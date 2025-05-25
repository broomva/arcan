"""Tests for the Arcan CLI."""

from typer.testing import CliRunner

from arcan import __version__
from arcan.cli import app

runner = CliRunner()


def test_version():
    """Test version display."""
    result = runner.invoke(app, ["--version"])
    assert result.exit_code == 0
    assert f"Arcan version: {__version__}" in result.stdout


def test_cast_spell():
    """Test casting a spell."""
    result = runner.invoke(app, ["cast", "transform", "--power", "3"])
    assert result.exit_code == 0
    assert "Casting spell: transform" in result.stdout
    assert "Power level: 3" in result.stdout


def test_cast_spell_default_power():
    """Test casting a spell with default power."""
    result = runner.invoke(app, ["cast", "optimize"])
    assert result.exit_code == 0
    assert "Casting spell: optimize" in result.stdout
    assert "Power level: 1" in result.stdout


def test_list_spells():
    """Test listing available spells."""
    result = runner.invoke(app, ["list-spells"])
    assert result.exit_code == 0
    assert "Available Spells" in result.stdout
    assert "transform" in result.stdout
    assert "analyze" in result.stdout
    assert "protect" in result.stdout
    assert "optimize" in result.stdout
    assert "document" in result.stdout


def test_help():
    """Test help display."""
    result = runner.invoke(app, ["--help"])
    assert result.exit_code == 0
    assert "Arcan - A powerful spellbook for modern development" in result.stdout
