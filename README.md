Stream a video dump of a [scrcpy] capture over TCP as if it was captured in
real-time.

[scrcpy]: https://github.com/Genymobile/scrcpy

This is useful for debugging a [player] consuming a stream from `scrcpy-server`.

[player]: https://code.videolan.org/rom1v/vlc/-/merge_requests/20

### Dump a stream

To capture a video stream (in the scrcpy protocol format) from an Android
device, download [`scrcpy-server`], plug an Android device, and execute:

[`scrcpy-server`]: https://github.com/Genymobile/scrcpy/releases/download/v1.25/scrcpy-server-v1.25

```bash
adb push scrcpy-server-v1.25 /data/local/tmp/scrcpy-server-manual.jar
adb forward tcp:1234 localabstract:scrcpy
adb shell CLASSPATH=/data/local/tmp/scrcpy-server-manual.jar \
    app_process / com.genymobile.scrcpy.Server 1.25 tunnel_forward=true \
    control=false cleanup=false send_device_meta=false send_frame_meta=true \
    send_dummy_byte=false max_size=1920
```

It will wait until a TCP client is connected to localhost:1234 to start
capturing.

In a separate terminal, connect to it and dump the whole TCP connection to a
file:

```bash
nc localhost 1234 > stream.scrcpy
```

Interrupt the capture with <kbd>Ctrl</kbd>+<kbd>c</kbd>.


### Re-stream a dump

First, build the project:

```bash
cargo build --release
```

Then start the streamer on a specific port with a specific dump file:

```bash
# Syntax: rtstreamer <port> <file>
target/release/rtstreamer 1235 stream.scrcpy
```

It will start streaming as soon as a TCP client connects to this port
(`localhost:1235` in the example).
