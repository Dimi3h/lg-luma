---
'lg-luma': minor
---

New Settings › System › Repair Bluetooth controller (on by default): a workaround for BLE gamepads that send a GATT Service Changed indication at their first button press, after which LG's Bluetooth stack stops passing on their input. Luma watches the controller's GATT cache file and, when the stack rewrites it, keeps the link up and restarts only the controller's HID connection, so its buttons work again. Measured with a Stadia controller on an LG OLED55B46LA (webOS 25) only. The first button press after each connection is lost, and LG shows its own disconnected and connected notices during the repair.

New Settings › System › Screen Share listening off (on by default): keeps the TV's Wi-Fi Direct listening disabled, because while it listens the Wi-Fi leaves the network for 100 ms twice a second, which made streaming and cloud gaming stutter (measured on an LG OLED55B46LA, webOS 25, only). LG's Screen Share app is left alone while it is in front, and turning the setting off switches listening back on if Luma switched it off. Miracast devices may not find the TV while it is off; AirPlay is not affected.
