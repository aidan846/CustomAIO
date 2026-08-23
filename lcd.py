"""Render system temperatures and upload them to a liquidctl-supported LCD.

Run ``python lcd.py --list-devices`` to discover devices, or use
``setup.bat`` to configure fan control, LCD control, and optional auto-start.
"""

from __future__ import annotations

import argparse
import json
import logging
from logging.handlers import RotatingFileHandler
import re
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from liquidctl import find_liquidctl_devices
from liquidctl.driver.base import BaseDriver
from liquidctl.driver.usb import PyUsbDevice


# ============================================================
# CONFIG DEFAULTS
# ============================================================

BACKGROUND = "#000000"
WHITE = "#FFFFFF"
ACCENT = "#FF7A1A"
RED_BG = "#FF0000"
DARK_BAR = "#202020"

CPU_OVERHEAT_TEMP = 85.0
GPU_OVERHEAT_TEMP = 85.0
TEMP_BAR_MIN = 20
TEMP_BAR_MAX = 90
SCREEN_ROTATION = 90
UPDATE_INTERVAL = 1

BASE_SIZE = (320, 320)
SCRIPT_DIR = Path(__file__).resolve().parent
DATA_DIR = SCRIPT_DIR / "data"
LHM_DIR = SCRIPT_DIR / "deps" / "lhm"
DATA_DIR.mkdir(parents=True, exist_ok=True)
CONFIG_PATH = DATA_DIR / "lcd_config.json"
FAN_CONFIG_PATH = DATA_DIR / "fan_config.bat"
OUTPUT_PATH = DATA_DIR / "lcd_frame.png"
LOG_PATH = DATA_DIR / "lcd.log"

DEFAULT_CONFIG = {
    # The defaults preserve the original Kraken Z53 behavior. setup.bat
    # replaces these with stable IDs for another selected device.
    "device_match": "Kraken Z",
    "device_pick": 1,
    "vendor_id": None,
    "product_id": None,
    "serial_number": None,
    "lcd_channel": "lcd",
    "lcd_mode": "static",
    "width": None,
    "height": None,
    "usb_interface": 0,
    "pyusb_fallback": True,
    "screen_rotation": SCREEN_ROTATION,
    "update_interval": UPDATE_INTERVAL,
    "cpu_temperature_source": "librehardwaremonitor",
    "cpu_vendor": "auto",
    "gpu_temperature_source": "nvidia-smi",
    "gpu_vendor": "nvidia",
    # Commands must print a temperature as their first number. The CPU command
    # is intentionally blank: reporting N/A is safer than a fake temperature.
    "cpu_temperature_command": "",
    "gpu_temperature_command": (
        "nvidia-smi --query-gpu=temperature.gpu "
        "--format=csv,noheader,nounits"
    ),
}


def configure_logging(console: bool = True) -> None:
    handlers: list[logging.Handler] = [
        RotatingFileHandler(
            LOG_PATH,
            maxBytes=512_000,
            backupCount=2,
            encoding="utf-8",
        )
    ]
    if console:
        handlers.append(logging.StreamHandler())

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s | %(levelname)s | %(message)s",
        handlers=handlers,
        force=True,
    )


def load_config() -> dict:
    config = DEFAULT_CONFIG.copy()
    if CONFIG_PATH.exists():
        try:
            saved = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
            if not isinstance(saved, dict):
                raise ValueError("the root value must be an object")
            config.update(saved)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            raise RuntimeError(f"Could not read {CONFIG_PATH.name}: {error}") from error
    return config


def _device_value(device, name, default=None):
    return getattr(getattr(device, "device", None), name, default)


def _hex_id(value) -> str:
    return f"0x{value:04X}" if isinstance(value, int) else "unknown"


def discover_devices() -> list:
    """Find only liquidctl devices that CustomAIO can actually control."""
    devices = []
    for device in find_liquidctl_devices():
        speed_channels = getattr(device, "_speed_channels", None)
        has_speed_channel = isinstance(speed_channels, dict) and bool(speed_channels)
        has_lcd = has_screen_api(device) and bool(getattr(device, "lcd_resolution", None))
        if has_speed_channel or has_lcd:
            devices.append(device)
    return devices


def has_screen_api(device) -> bool:
    """Return whether the driver overrides liquidctl's base screen method."""
    return type(device).set_screen is not BaseDriver.set_screen


def recommended_device_number(devices: list, purpose: str) -> int:
    """Prefer a device with the capability required by the setup stage."""
    for index, device in enumerate(devices, start=1):
        speed_channels = getattr(device, "_speed_channels", None)
        if purpose == "fan" and isinstance(speed_channels, dict) and speed_channels:
            return index
        if purpose == "lcd" and getattr(device, "lcd_resolution", None):
            return index
    return 1


def list_devices(devices: list) -> None:
    if not devices:
        print("No liquidctl-supported devices were found.")
        return

    print("liquidctl devices:\n")
    for index, device in enumerate(devices, start=1):
        resolution = getattr(device, "lcd_resolution", None)
        speed_channels = getattr(device, "_speed_channels", None)
        if isinstance(speed_channels, dict):
            speed_text = ", ".join(speed_channels) or "not reported"
        else:
            speed_text = "not reported; see the liquidctl device guide"
        resolution_text = (
            f"{resolution[0]}x{resolution[1]}" if resolution else "not reported"
        )
        print(f"[{index}] {device.description}")
        print(
            f"    VID {_hex_id(_device_value(device, 'vendor_id'))} | "
            f"PID {_hex_id(_device_value(device, 'product_id'))}"
        )
        print(f"    Serial: {_device_value(device, 'serial_number') or 'not reported'}")
        print(f"    LCD resolution: {resolution_text}")
        print(f"    Speed channels: {speed_text}")
        print(f"    Screen API: {'driver-specific' if has_screen_api(device) else 'none'}")

    print(
        "\nA driver-specific screen API does not guarantee static-image support. "
        "Check the device guide in liquidctl's documentation."
    )


def save_device_config(
    device,
    cpu_source: str = "librehardwaremonitor",
    cpu_vendor: str = "auto",
    gpu_source: str = "nvidia-smi",
    gpu_vendor: str = "nvidia",
) -> None:
    resolution = getattr(device, "lcd_resolution", None)
    config = DEFAULT_CONFIG.copy()
    config.update(
        {
            "device_match": device.description,
            "device_pick": 1,
            "vendor_id": _device_value(device, "vendor_id"),
            "product_id": _device_value(device, "product_id"),
            "serial_number": _device_value(device, "serial_number"),
            "width": resolution[0] if resolution else None,
            "height": resolution[1] if resolution else None,
            "cpu_temperature_source": cpu_source,
            "cpu_vendor": cpu_vendor,
            "gpu_temperature_source": gpu_source,
            "gpu_vendor": gpu_vendor,
        }
    )
    CONFIG_PATH.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    print(f"Saved LCD configuration to {CONFIG_PATH}")


def save_fan_config(
    device,
    fan_channel: str,
    fan_control: str,
    pump_channel: str,
    pump_control: str,
) -> None:
    def batch_value(value) -> str:
        return str(value or "").replace("%", "%%").replace('"', "")

    vendor_id = _device_value(device, "vendor_id")
    product_id = _device_value(device, "product_id")
    lines = [
        "@rem Generated by setup.bat. Run setup.bat again to replace this file.",
        f'set "DEVICE_MATCH={batch_value(device.description)}"',
        f'set "DEVICE_SERIAL={batch_value(_device_value(device, "serial_number"))}"',
        f'set "DEVICE_VENDOR={vendor_id:04X}"' if vendor_id is not None else 'set "DEVICE_VENDOR="',
        f'set "DEVICE_PRODUCT={product_id:04X}"' if product_id is not None else 'set "DEVICE_PRODUCT="',
        f'set "FAN_CHANNEL={batch_value(fan_channel)}"',
        f'set "FAN_CONTROL={batch_value(fan_control)}"',
        f'set "PUMP_CHANNEL={batch_value(pump_channel)}"',
        f'set "PUMP_CONTROL={batch_value(pump_control)}"',
        'set "LIQUIDCTL_EXTRA="',
    ]
    FAN_CONFIG_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Saved fan configuration to {FAN_CONFIG_PATH}")


def select_device(devices: list, config: dict):
    candidates = devices

    serial = config.get("serial_number")
    if serial:
        candidates = [
            device
            for device in candidates
            if str(_device_value(device, "serial_number", "")).casefold()
            == str(serial).casefold()
        ]

    vendor_id = config.get("vendor_id")
    if vendor_id is not None:
        candidates = [
            device
            for device in candidates
            if _device_value(device, "vendor_id") == int(vendor_id)
        ]

    product_id = config.get("product_id")
    if product_id is not None:
        candidates = [
            device
            for device in candidates
            if _device_value(device, "product_id") == int(product_id)
        ]

    match = str(config.get("device_match") or "").casefold()
    if match:
        candidates = [
            device for device in candidates if match in device.description.casefold()
        ]

    if not candidates:
        raise RuntimeError(
            "The configured LCD device was not found. Run setup.bat and "
            "choose Identify/configure LCD device again."
        )

    pick = int(config.get("device_pick") or 1)
    if pick < 1 or pick > len(candidates):
        raise RuntimeError(
            f"device_pick is {pick}, but only {len(candidates)} matching device(s) exist"
        )

    return candidates[pick - 1]


# ============================================================
# LCD GRAPHICS
# ============================================================

def load_font(size: int):
    try:
        return ImageFont.truetype(r"C:\Windows\Fonts\segoeuib.ttf", size)
    except Exception:
        return ImageFont.load_default()


def draw_centered(draw, text, y, font, fill=WHITE):
    box = draw.textbbox((0, 0), text, font=font)
    x = (BASE_SIZE[0] - (box[2] - box[0])) // 2
    draw.text((x, y), text, font=font, fill=fill)


def draw_temp_bar(draw, y: int, temp: float | None):
    x1, x2, height = 75, 245, 14
    percentage = 0.0
    if temp is not None:
        clamped = max(TEMP_BAR_MIN, min(TEMP_BAR_MAX, temp))
        percentage = (clamped - TEMP_BAR_MIN) / (TEMP_BAR_MAX - TEMP_BAR_MIN)

    radius = height // 2
    draw.rounded_rectangle((x1, y, x2, y + height), radius=radius, fill=DARK_BAR)
    if percentage > 0:
        fill_x = x1 + int((x2 - x1) * percentage)
        draw.rounded_rectangle(
            (x1, y, fill_x, y + height), radius=radius, fill=ACCENT
        )

    range_font = load_font(11)
    draw.text((x1, y + height + 4), f"{TEMP_BAR_MIN}°C", font=range_font, fill=WHITE)
    right_text = f"{TEMP_BAR_MAX}°C"
    box = draw.textbbox((0, 0), right_text, font=range_font)
    draw.text((x2 - (box[2] - box[0]), y + height + 4), right_text, font=range_font, fill=WHITE)


def _temperature_text(temp: float | None) -> str:
    return f"{round(temp)}°" if temp is not None else "N/A"


def make_frame(
    cpu_temp: float | None,
    gpu_temp: float | None,
    size: tuple[int, int],
    rotation: int,
):
    if gpu_temp is not None and gpu_temp >= GPU_OVERHEAT_TEMP:
        component_name = "GPU"
    elif cpu_temp is not None and cpu_temp >= CPU_OVERHEAT_TEMP:
        component_name = "CPU"
    else:
        component_name = None

    image = Image.new("RGB", BASE_SIZE, RED_BG if component_name else BACKGROUND)
    draw = ImageDraw.Draw(image)

    if component_name:
        title_font = load_font(28)
        sub_font = load_font(22)
        draw_centered(draw, f"{component_name} IS", 90, title_font)
        draw_centered(draw, "TOO HOT!!!", 130, title_font)
        draw_centered(draw, "Give it a break.", 190, sub_font)
    else:
        label_font = load_font(18)
        temp_font = load_font(48)
        draw_centered(draw, "CPU", 30, label_font)
        draw_centered(draw, _temperature_text(cpu_temp), 50, temp_font)
        draw_temp_bar(draw, 112, cpu_temp)
        draw_centered(draw, "GPU", 165, label_font)
        draw_centered(draw, _temperature_text(gpu_temp), 185, temp_font)
        draw_temp_bar(draw, 247, gpu_temp)

    rotation = int(rotation) % 360
    if rotation:
        image = image.rotate(rotation, expand=False)
    if size != BASE_SIZE:
        image = image.resize(size, Image.Resampling.LANCZOS)
    return image


# ============================================================
# TEMPERATURES AND USB
# ============================================================

def read_temperature(command: str) -> float | None:
    if not command.strip():
        return None
    try:
        result = subprocess.check_output(
            command,
            text=True,
            stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            timeout=8,
        )
        match = re.search(r"-?\d+(?:\.\d+)?", result)
        return float(match.group()) if match else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


_LHM_WARNINGS_SHOWN: set[str] = set()
_LHM_COMPUTER = None


def _get_lhm_computer():
    """Load the project's matched LibreHardwareMonitor assembly bundle."""
    global _LHM_COMPUTER
    if _LHM_COMPUTER is not None:
        return _LHM_COMPUTER
    library = LHM_DIR / "LibreHardwareMonitorLib.dll"
    ram_spd = LHM_DIR / "RAMSPDToolkit-NDD.dll"
    if not library.exists() or not ram_spd.exists():
        raise RuntimeError("LibreHardwareMonitor files are missing from deps\\lhm")
    import pythonnet
    pythonnet.load("coreclr")
    import clr
    clr.AddReference(str(library))
    from LibreHardwareMonitor.Hardware import Computer
    computer = Computer()
    computer.IsCpuEnabled = True
    computer.IsGpuEnabled = True
    computer.IsMotherboardEnabled = False
    computer.IsMemoryEnabled = False
    computer.IsStorageEnabled = False
    computer.IsControllerEnabled = False
    computer.IsNetworkEnabled = False
    computer.Open()
    _LHM_COMPUTER = computer
    return computer


def _walk_hardware(hardware):
    hardware.Update()
    yield hardware
    for child in hardware.SubHardware:
        yield from _walk_hardware(child)


def read_lhm_temperature(component: str) -> float | None:
    """Read the hottest CPU/GPU temperature from CustomAIO's local LHM bundle."""
    try:
        values = []
        for root in _get_lhm_computer().Hardware:
            for hardware in _walk_hardware(root):
                if component == "cpu" and "Cpu" not in str(hardware.HardwareType):
                    continue
                if component == "gpu" and "Gpu" not in str(hardware.HardwareType):
                    continue
                for sensor in hardware.Sensors:
                    if "Temperature" in str(sensor.SensorType):
                        # Intel exposes "Distance to TjMax" as a Temperature
                        # sensor.  It is thermal headroom, not an actual CPU
                        # temperature, so including it can make the LCD report
                        # a value around 70 C while the cores are near 40 C.
                        sensor_name = str(sensor.Name).casefold()
                        if component == "cpu" and "distance to tjmax" in sensor_name:
                            continue
                        value = float(sensor.Value) if sensor.Value is not None else None
                        if value is not None and -20 <= value <= 150:
                            values.append(value)
        if values:
            return max(values)
        if component not in _LHM_WARNINGS_SHOWN:
            logging.warning(
                "LibreHardwareMonitor found no usable %s temperature values; "
                "the CPU/GPU sensor driver may be blocked or unsupported.",
                component,
            )
            _LHM_WARNINGS_SHOWN.add(component)
        return None
    except Exception as error:
        if component not in _LHM_WARNINGS_SHOWN:
            logging.warning("LibreHardwareMonitor %s temperature is unavailable: %s", component, error)
            _LHM_WARNINGS_SHOWN.add(component)
        return None


def read_configured_temperatures(config: dict) -> tuple[float | None, float | None]:
    cpu_source = str(config.get("cpu_temperature_source", "librehardwaremonitor")).casefold()
    gpu_source = str(config.get("gpu_temperature_source", "nvidia-smi")).casefold()
    cpu_command = str(config.get("cpu_temperature_command", ""))
    gpu_command = str(config.get("gpu_temperature_command", ""))

    cpu_temp = read_temperature(cpu_command) if cpu_command.strip() else None
    gpu_temp = None
    if gpu_source == "nvidia-smi":
        gpu_temp = read_temperature(gpu_command or DEFAULT_CONFIG["gpu_temperature_command"])

    # Treat legacy WinTmp settings as the bundled LibreHardwareMonitor source.
    if cpu_source in ("wintmp", "librehardwaremonitor") and cpu_temp is None:
        cpu_temp = read_lhm_temperature("cpu")
    if gpu_source in ("wintmp", "librehardwaremonitor"):
        gpu_temp = read_lhm_temperature("gpu")

    return cpu_temp, gpu_temp


def attach_pyusb_fallback(device, config: dict):
    """Attach a dynamic PyUSB bulk device for Kraken-family Windows setups."""
    if getattr(device, "bulk_device", None) is not None:
        return None
    if not config.get("pyusb_fallback", True):
        return None
    if not type(device).__module__.endswith(".kraken3"):
        return None

    vendor_id = _device_value(device, "vendor_id")
    product_id = _device_value(device, "product_id")
    if vendor_id is None or product_id is None:
        return None

    expected_serial = _device_value(device, "serial_number")
    candidates = list(PyUsbDevice.enumerate(vendor_id, product_id))
    if expected_serial:
        serial_matches = []
        for item in candidates:
            try:
                if item.serial_number == expected_serial:
                    serial_matches.append(item)
            except (ValueError, OSError) as error:
                logging.debug("Could not read PyUSB serial number: %s", error)
        if serial_matches:
            candidates = serial_matches
        else:
            logging.info(
                "PyUSB serial matching was unavailable; using the first matching USB device"
            )
    if not candidates:
        logging.warning("PyUSB found no matching bulk interface")
        return None

    bulk = candidates[0]
    interface = int(config.get("usb_interface", 0))
    logging.info("Using PyUSB fallback on interface %s", interface)
    bulk.open(bInterfaceNumber=interface)
    bulk.claim()
    device.bulk_device = bulk
    return bulk


def _display_temp(temp: float | None) -> str:
    return "N/A" if temp is None else f"{temp:.0f}C"


def run_service(config: dict) -> None:
    device = select_device(discover_devices(), config)
    if not has_screen_api(device):
        raise RuntimeError(f"{device.description} does not expose liquidctl's screen API")

    reported_size = getattr(device, "lcd_resolution", None)
    width = int(config.get("width") or (reported_size[0] if reported_size else BASE_SIZE[0]))
    height = int(config.get("height") or (reported_size[1] if reported_size else BASE_SIZE[1]))
    size = (width, height)
    interval = max(1.0, float(config.get("update_interval", UPDATE_INTERVAL)))

    logging.info("Found LCD device: %s", device.description)
    logging.info("Rendering %sx%s every %s seconds", width, height, interval)

    bulk = None
    try:
        bulk = attach_pyusb_fallback(device, config)
        with device.connect():
            while True:
                cpu_temp, gpu_temp = read_configured_temperatures(config)
                frame = make_frame(
                    cpu_temp,
                    gpu_temp,
                    size,
                    int(config.get("screen_rotation", SCREEN_ROTATION)),
                )
                frame.save(OUTPUT_PATH)
                device.set_screen(
                    str(config.get("lcd_channel", "lcd")),
                    str(config.get("lcd_mode", "static")),
                    str(OUTPUT_PATH),
                )
                logging.info(
                    "LCD updated | CPU %s | GPU %s",
                    _display_temp(cpu_temp),
                    _display_temp(gpu_temp),
                )
                time.sleep(interval)
    except KeyboardInterrupt:
        logging.info("Stopping LCD service")
    finally:
        if bulk:
            try:
                bulk.close()
            except Exception:
                logging.debug("Could not close PyUSB fallback cleanly", exc_info=True)


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list-devices", action="store_true")
    parser.add_argument("--recommended-device", choices=("fan", "lcd"))
    parser.add_argument(
        "--configure",
        type=int,
        metavar="NUMBER",
        help="save the numbered device from --list-devices to lcd_config.json",
    )
    parser.add_argument(
        "--configure-fan",
        type=int,
        metavar="NUMBER",
        help="save a fan_config.bat selector for the numbered liquidctl device",
    )
    parser.add_argument(
        "--cpu-source",
        choices=("none", "librehardwaremonitor"),
        default="librehardwaremonitor",
    )
    parser.add_argument("--cpu-vendor", choices=("auto", "intel", "amd"), default="auto")
    parser.add_argument(
        "--gpu-source",
        choices=("none", "nvidia-smi", "librehardwaremonitor"),
        default="nvidia-smi",
    )
    parser.add_argument(
        "--gpu-vendor",
        choices=("auto", "nvidia", "amd", "intel"),
        default="nvidia",
    )
    parser.add_argument("--fan-channel", default="fan")
    parser.add_argument(
        "--fan-control",
        choices=("curve", "fixed", "disabled"),
        default="curve",
    )
    parser.add_argument("--pump-channel", default="pump")
    parser.add_argument(
        "--pump-control",
        choices=("curve", "fixed", "disabled"),
        default="curve",
    )
    parser.add_argument(
        "--render-test",
        action="store_true",
        help="render a sample image without accessing an LCD device",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    configure_logging(console=sys.stdout is not None)

    try:
        if args.list_devices:
            list_devices(discover_devices())
            return 0
        if args.recommended_device:
            print(recommended_device_number(discover_devices(), args.recommended_device))
            return 0
        if args.configure is not None:
            devices = discover_devices()
            if args.configure < 1 or args.configure > len(devices):
                raise RuntimeError(f"Choose a device number from 1 to {len(devices)}")
            save_device_config(
                devices[args.configure - 1],
                args.cpu_source,
                args.cpu_vendor,
                args.gpu_source,
                args.gpu_vendor,
            )
            return 0
        if args.configure_fan is not None:
            devices = discover_devices()
            if args.configure_fan < 1 or args.configure_fan > len(devices):
                raise RuntimeError(f"Choose a device number from 1 to {len(devices)}")
            save_fan_config(
                devices[args.configure_fan - 1],
                args.fan_channel,
                args.fan_control,
                args.pump_channel,
                args.pump_control,
            )
            return 0
        if args.render_test:
            config = load_config()
            size = (
                int(config.get("width") or BASE_SIZE[0]),
                int(config.get("height") or BASE_SIZE[1]),
            )
            make_frame(42, 55, size, int(config.get("screen_rotation", 0))).save(
                OUTPUT_PATH
            )
            print(f"Rendered {OUTPUT_PATH}")
            return 0

        run_service(load_config())
        return 0
    except Exception:
        logging.exception("LCD service stopped because of an error")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
