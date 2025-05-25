"""Arcan CLI - Command line interface for the Arcan spellbook."""

import typer
from rich.console import Console
from rich.panel import Panel

from arcan import __version__

app = typer.Typer(
    name="arcan",
    help="Arcan - A powerful spellbook for modern development",
    add_completion=True,
)
console = Console()


def version_callback(value: bool) -> None:
    """Handle version display."""
    if value:
        console.print(f"Arcan version: {__version__}")
        raise typer.Exit


@app.callback()
def main(
    version: bool | None = typer.Option(
        None,
        "--version",
        "-v",
        callback=version_callback,
        is_eager=True,
        help="Show the application version and exit.",
    ),
) -> None:
    """Arcan - A powerful spellbook for modern development."""


@app.command()
def cast(
    spell: str = typer.Argument(..., help="The spell to cast"),
    power: int = typer.Option(1, "--power", "-p", help="The power level of the spell"),
) -> None:
    """Cast a spell from the Arcan spellbook."""
    console.print(
        Panel(
            f"[bold cyan]Casting spell:[/bold cyan] {spell}\n"
            f"[bold yellow]Power level:[/bold yellow] {power}",
            title="🪄 Arcan Spellcaster",
            border_style="bright_blue",
        ),
    )


@app.command()
def list_spells() -> None:
    """List all available spells in the Arcan spellbook."""
    spells = [
        "✨ transform - Transform code between formats",
        "🔍 analyze - Analyze code structure and patterns",
        "🛡️ protect - Add security and validation layers",
        "⚡ optimize - Optimize code performance",
        "📚 document - Generate documentation",
    ]

    console.print(
        Panel(
            "\n".join(spells),
            title="📖 Available Spells",
            border_style="bright_magenta",
        ),
    )


if __name__ == "__main__":
    app()
