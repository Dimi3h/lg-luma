---
'lg-luma': minor
---

New Settings › System › Repair Bluetooth controller (on by default): a workaround for BLE gamepads that send a GATT Service Changed indication at their first button press (and again after idling), after which LG's Bluetooth stack stops passing on their input. Luma watches the controller's GATT cache file and, every time the stack rewrites it, keeps the link up and restarts only the controller's HID connection, so its buttons work again. Measured with a Stadia controller on an LG OLED55B46LA (webOS 25) only. The first button press after each connection is lost, and LG shows its own disconnected and connected notices during the repair.

New Settings › System › Screen Share listening off (on by default): keeps the TV's Wi-Fi Direct listening disabled, because while it listens the Wi-Fi leaves the network for 100 ms twice a second, which made streaming and cloud gaming stutter (measured on an LG OLED55B46LA, webOS 25, only). LG's Screen Share app is left alone while it is in front, and turning the setting off switches listening back on if Luma switched it off. Miracast devices may not find the TV while it is off; AirPlay is not affected.

New Settings › System › Stadia controller as Xbox controller (on by default, with the Home takeover): the elevated helper presents the Stadia controller as a virtual Xbox 360 pad, started at every cold boot so it exists before a game starts, so its analog triggers work in LG's browser (GeForce NOW); the real controller is hidden from apps while the bridge runs, so the browser does not see it as a second pad. Measured on an LG OLED55B46LA (webOS 25) only; vibration is not passed on in this mode.
