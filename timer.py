#!/usr/bin/env python3
import subprocess
import asyncio
from textual.app import App, ComposeResult
from textual.widgets import (
    Static, Input, Checkbox, Button, Footer, Header, Select
)
from textual.containers import Vertical, Horizontal, Center


class TimerTUI(App):
    CSS = """
    Screen {
        align: center middle;
        background: #101010;
    }

    #main {
        width: 50%;
        border: solid #33ccff;
        padding: 1 2;
        background: #181818;
        border-title-align: center;
        border-title-color: cyan;
    }

    .label {
        color: #33ccff;
        text-align: center;
        height: auto;
    }

    Input, Select {
        width: 100%;
        border: round #444;
        padding: 0 1;
    }

    Checkbox {
        width: 100%;
    }

    Button {
        width: 48%;
        margin: 0 1;
    }

    #status {
        color: cyan;
        text-align: center;
        padding-top: 1;
    }
    """

    def __init__(self):
        super().__init__()
        self.timer_process = None
        self.message_label = Static("", id="status")

    def compose(self) -> ComposeResult:
        yield Header()
        with Center():
            with Vertical(id="main"):
                yield Static("⏱ TIMER CONFIGURATION ⏱", classes="label")

                self.input_time = Input(value="", placeholder="Время (например 20m)")
                self.cb_workspace = Checkbox("Переключить воркспейс", value=True)
                self.input_workspace = Input(value="12", placeholder="Номер воркспейса")

                self.cb_player = Checkbox("Управлять плеером", value=True)
                self.select_player_action = Select(
                    options=[
                        ("Ничего", "none"),
                        ("stop", "stop"),
                        ("pause", "pause"),
                        ("next", "next"),
                        ("previous", "previous"),
                    ],
                    value="stop",
                )

                self.cb_lock = Checkbox("Заблокировать экран", value=True)
                self.cb_kill = Checkbox("Убить приложения", value=False)
                self.input_kill = Input(value="firefox, telegram-desktop", placeholder="через запятую")

                yield self.input_time
                yield self.cb_workspace
                yield self.input_workspace
                yield self.cb_player
                yield self.select_player_action
                yield self.cb_lock
                yield self.cb_kill
                yield self.input_kill

                with Horizontal():
                    yield Button("▶ Start / Restart Timer", id="start", variant="success")
                    yield Button("✖ Quit", id="quit", variant="error")

                yield self.message_label

        yield Footer()

    async def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "quit":
            await self.stop_timer()
            await self.action_quit()
        elif event.button.id == "start":
            await self.restart_timer()

    async def stop_timer(self):
        if self.timer_process and self.timer_process.poll() is None:
            self.timer_process.terminate()
            await asyncio.sleep(0.2)
            if self.timer_process.poll() is None:
                self.timer_process.kill()

    async def restart_timer(self):
        await self.stop_timer()
        self.notify("⏳ Запуск таймера...")

        time_str = self.input_time.value.strip()
        commands = []

        # Workspace switch
        if self.cb_workspace.value:
            workspace = self.input_workspace.value.strip()
            commands.append(f"niri msg action focus-workspace {workspace}")

        # Player action
        if self.cb_player.value:
            action = self.select_player_action.value
            if action != "none":
                commands.append(f"playerctl {action}")

        # Lock screen
        if self.cb_lock.value:
            commands.append("qs -c noctalia-shell ipc call lockScreen lock")

        # Kill apps
        if self.cb_kill.value and self.input_kill.value.strip():
            for proc in self.input_kill.value.split(","):
                proc = proc.strip()
                if proc:
                    commands.append(f"pkill {proc}")

        cmd_after = " ; ".join(commands)
        full_cmd = f"timer {time_str} && bash -c '{cmd_after}'"

        self.timer_process = subprocess.Popen(
            full_cmd,
            shell=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self.notify(f"✅ Таймер запущен на {time_str}")

    def notify(self, msg: str):
        self.message_label.update(msg)


if __name__ == "__main__":
    TimerTUI().run()
