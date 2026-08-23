import subprocess
import time
import logging
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from liquidctl import find_liquidctl_devices
from liquidctl.driver.usb import PyUsbDevice


# ============================================================
# CONFIG
# ============================================================

WIDTH = 320
HEIGHT = 320

BACKGROUND = "#000000"
WHITE = "#FFFFFF"

# Brighter orange
ORANGE = "#FF7A1A"

# Overheat warning colors & thresholds
RED_BG = "#FF0000"
CPU_OVERHEAT_TEMP = 85.0
GPU_OVERHEAT_TEMP = 85.0

# Temperature bar scale limits (Min/Max shown on the UI)
TEMP_BAR_MIN = 20
TEMP_BAR_MAX = 90

# Screen rotation in degrees (PIL uses counter-clockwise, 
# so 90 or -90 rotates it accordingly)
SCREEN_ROTATION = 90

DARK_BAR = "#202020"

UPDATE_INTERVAL = 10

SCRIPT_DIR = Path(__file__).resolve().parent
OUTPUT_PATH = SCRIPT_DIR / "lcd_frame.png"
LOG_PATH = SCRIPT_DIR / "log.txt"
LHM_DIR = SCRIPT_DIR / "deps" / "lhm"
logging.basicConfig(
    filename=LOG_PATH,
    level=logging.INFO,
    format="%(asctime)s | %(levelname)s | %(message)s",
)
_LHM_COMPUTER = None


# ============================================================
# LCD GRAPHICS
# ============================================================

def load_font(size):
    path = r"C:\Windows\Fonts\segoeuib.ttf"

    try:
        return ImageFont.truetype(path, size)
    except Exception:
        return ImageFont.load_default()


def draw_centered(draw, text, y, font, fill=WHITE):
    box = draw.textbbox(
        (0, 0),
        text,
        font=font,
    )

    text_width = box[2] - box[0]
    x = (WIDTH - text_width) // 2

    draw.text(
        (x, y),
        text,
        font=font,
        fill=fill,
    )


def draw_temp_bar(draw, y, temp):
    # Shorter / narrower bar
    x1 = 75
    x2 = 245

    # Slightly thicker
    height = 14

    min_temp = TEMP_BAR_MIN
    max_temp = TEMP_BAR_MAX

    # Clamp actual temperature to bar range
    clamped_temp = max(
        min_temp,
        min(max_temp, temp),
    )

    # Convert actual temperature into 0-100%
    percentage = (
        clamped_temp - min_temp
    ) / (
        max_temp - min_temp
    )

    fill_width = int(
        (x2 - x1) * percentage
    )

    fill_x = x1 + fill_width

    radius = height // 2

    # Empty/background bar
    draw.rounded_rectangle(
        (
            x1,
            y,
            x2,
            y + height,
        ),
        radius=radius,
        fill=DARK_BAR,
    )

    # Filled orange section
    if percentage > 0:
        draw.rounded_rectangle(
            (
                x1,
                y,
                fill_x,
                y + height,
            ),
            radius=radius,
            fill=ORANGE,
        )

    # Temperature range labels dynamically use our config variables
    range_font = load_font(11)

    draw.text(
        (x1, y + height + 4),
        f"{TEMP_BAR_MIN}°C",
        font=range_font,
        fill=WHITE,
    )

    right_text = f"{TEMP_BAR_MAX}°C"

    box = draw.textbbox(
        (0, 0),
        right_text,
        font=range_font,
    )

    right_width = box[2] - box[0]

    draw.text(
        (
            x2 - right_width,
            y + height + 4,
        ),
        right_text,
        font=range_font,
        fill=WHITE,
    )


def make_frame(cpu_temp, gpu_temp):
    # Check for overheating conditions (GPU takes priority if both are hot)
    if gpu_temp >= GPU_OVERHEAT_TEMP:
        bg_color = RED_BG
        is_overheat = True
        component_name = "GPU"
    elif cpu_temp >= CPU_OVERHEAT_TEMP:
        bg_color = RED_BG
        is_overheat = True
        component_name = "CPU"
    else:
        bg_color = BACKGROUND
        is_overheat = False

    image = Image.new(
        "RGB",
        (WIDTH, HEIGHT),
        bg_color,
    )

    draw = ImageDraw.Draw(image)

    if is_overheat:
        # Overheat warning layout
        title_font = load_font(28)
        sub_font = load_font(22)

        draw_centered(
            draw,
            f"{component_name} IS",
            90,
            title_font,
            fill=WHITE,
        )
        draw_centered(
            draw,
            "TOO HOT!!!",
            130,
            title_font,
            fill=WHITE,
        )
        draw_centered(
            draw,
            "Give it a break.",
            190,
            sub_font,
            fill=WHITE,
        )
    else:
        # Normal telemetry layout
        label_font = load_font(18)
        temp_font = load_font(48)

        # ========================================================
        # CPU
        # ========================================================

        draw_centered(
            draw,
            "CPU",
            30,
            label_font,
        )

        draw_centered(
            draw,
            f"{round(cpu_temp)}°",
            50,
            temp_font,
        )

        draw_temp_bar(
            draw,
            112,
            cpu_temp,
        )

        # ========================================================
        # GPU
        # ========================================================

        draw_centered(
            draw,
            "GPU",
            165,
            label_font,
        )

        draw_centered(
            draw,
            f"{round(gpu_temp)}°",
            185,
            temp_font,
        )

        draw_temp_bar(
            draw,
            247,
            gpu_temp,
        )

    # Rotate the ENTIRE rendered frame based on the config variable.
    image = image.rotate(
        SCREEN_ROTATION,
        expand=False,
    )

    return image


# ============================================================
# TEMPERATURES
# ============================================================

def get_gpu_temp():
    """
    Get NVIDIA GPU temperature using nvidia-smi.
    """

    try:
        result = subprocess.check_output(
            [
                "nvidia-smi",
                "--query-gpu=temperature.gpu",
                "--format=csv,noheader,nounits",
            ],
            text=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )

        return float(
            result.strip().splitlines()[0]
        )

    except Exception:
        return get_lhm_temp("gpu") or 0


def get_cpu_temp():
    """Read CPU temperature from the project's local LHM dependency bundle."""
    return get_lhm_temp("cpu") or 0


def get_lhm_computer():
    global _LHM_COMPUTER
    if _LHM_COMPUTER is not None:
        return _LHM_COMPUTER
    library = LHM_DIR / "LibreHardwareMonitorLib.dll"
    ram_spd = LHM_DIR / "RAMSPDToolkit-NDD.dll"
    if not library.exists() or not ram_spd.exists():
        raise RuntimeError("Missing LibreHardwareMonitor files in deps\\lhm")
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
    computer.Open()
    _LHM_COMPUTER = computer
    return computer


def get_lhm_temp(component):
    try:
        readings = []
        for hardware in get_lhm_computer().Hardware:
            hardware.Update()
            if component == "cpu" and "Cpu" not in str(hardware.HardwareType):
                continue
            if component == "gpu" and "Gpu" not in str(hardware.HardwareType):
                continue
            for sensor in hardware.Sensors:
                if "Temperature" in str(sensor.SensorType) and sensor.Value is not None:
                    value = float(sensor.Value)
                    if -20 <= value <= 150:
                        readings.append(value)
        return max(readings) if readings else None
    except Exception as error:
        logging.warning("%s temperature unavailable: %s", component, error)
        return None


# ============================================================
# KRAKEN
# ============================================================

def find_kraken():
    for device in find_liquidctl_devices():
        if "Kraken Z" in device.description:
            return device

    raise RuntimeError(
        "NZXT Kraken Z53 not found."
    )


def attach_bulk_device(kraken):
    print(
        "Searching PyUSB for Kraken LCD interface..."
    )

    devices = list(
        PyUsbDevice.enumerate(
            0x1E71,
            0x3008,
        )
    )

    if not devices:
        raise RuntimeError(
            "PyUSB could not locate the Kraken Z53."
        )

    print(
        f"Found {len(devices)} Kraken USB device(s)."
    )

    bulk = devices[0]

    print(
        "Opening Kraken interface 0 (LCD / WinUSB)..."
    )

    bulk.open(
        bInterfaceNumber=0
    )

    bulk.claim()

    kraken.bulk_device = bulk

    print(
        "Kraken LCD bulk interface attached successfully."
    )

    return bulk


# ============================================================
# MAIN
# ============================================================

def main():
    print("Finding Kraken Z53...")

    kraken = find_kraken()

    print(
        f"Found: {kraken.description}"
    )

    bulk = None

    try:
        bulk = attach_bulk_device(
            kraken
        )

        with kraken.connect():
            kraken.initialize()

            print()
            print("Kraken LCD service running.")
            print(
                f"Update interval: {UPDATE_INTERVAL} seconds"
            )
            print("Press Ctrl+C to stop.")
            print()

            while True:
                cpu_temp = get_cpu_temp()
                gpu_temp = get_gpu_temp()

                frame = make_frame(
                    cpu_temp,
                    gpu_temp,
                )

                frame.save(
                    OUTPUT_PATH
                )

                try:
                    kraken.set_screen(
                        "lcd",
                        "static",
                        str(OUTPUT_PATH),
                    )

                    print(
                        f"LCD updated | "
                        f"CPU {cpu_temp:.0f}C | "
                        f"GPU {gpu_temp:.0f}C"
                    )
                    logging.info("LCD updated | CPU %sC | GPU %sC", round(cpu_temp), round(gpu_temp))

                except Exception as error:
                    print(
                        f"LCD update failed: {error}"
                    )
                    logging.exception("LCD update failed")

                time.sleep(
                    UPDATE_INTERVAL
                )

    except KeyboardInterrupt:
        print()
        print(
            "Stopping Kraken LCD service..."
        )

    finally:
        if bulk:
            try:
                bulk.close()
            except Exception:
                pass

        print("Stopped.")


if __name__ == "__main__":
    main()
