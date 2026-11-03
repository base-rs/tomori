# Sharing a Tomori Space in a Slack Huddle

This is the Linux setup used for the November 2026 demo. It sends the spectator's 1080p view and the space's mixed audio to Slack **without OBS**. The video and audio devices are temporary: recreate them after a reboot or PipeWire restart. The camera bridge and spectator must remain running.

This is a one-way feed into Slack. Slack participants' voices are not automatically routed back into the Tomori space. The huddle hears the space's participant audio and agent speech; make sure everyone in the space knows it is being shared. Do not mix Slack's own output into the virtual microphone, or it may feed back into the huddle.

## Prerequisites

- A running Tomori server and Quest in the space to share. See the server and Quest instructions in the [main readme](../readme.md).
- A Linux desktop with PipeWire, its PulseAudio compatibility server, and ALSA's default output using the **native PipeWire ALSA plugin**. The PID mapping below was verified with that plugin, not with ALSA's PulseAudio plugin. The recipe also needs `pactl`, `pw-dump`, Python 3, `v4l2loopback` and `v4l2loopback-ctl`, `v4l2-ctl`, and GStreamer (`gst-launch-1.0`, `pipewiresrc`, `capssetter`, `queue`, `videoconvert`, and `filesink`). Creating a virtual camera and refreshing udev require administrative privileges.
- Slack's desktop application, with permission to use the chosen camera and microphone.

Do not unload or reload `v4l2loopback` while another application is using its cameras. The commands below create a separate device.

## 1. Start the spectator

From the repository root, build the client and discover the target space:

```sh
cargo build -p client-quest --bin tomori
./target/debug/tomori discover --server 127.0.0.1:8765
```

Use the actual server address and space ID from the output. Run this in a terminal and leave it running:

```sh
./target/debug/tomori --spectator-pipewire \
  --server 127.0.0.1:8765 --space SPACE_ID
```

The spectator publishes `tomori-spectator` as a 1920×1080 RGBA PipeWire video source at up to 30 fps. It follows the first participant reporting presence, or use `--follow NAME` to select one. The Quest must be connected to the same space and reporting presence. After a server restart, space IDs change: discover and select the new one.

## 2. Create and feed a virtual camera

The `v4l2loopback` module must already be loaded. Create one camera and note the device path it prints:

```sh
sudo v4l2loopback-ctl add -n 'Tomori Spectator' -x 1
udevadm settle
```

Substitute that exact path below; do not assume `/dev/video3` on another machine. The user running GStreamer and Slack needs read/write access to the device.

```sh
CAMERA=/dev/video3  # replace with the path printed above
exec 3<>"$CAMERA"
v4l2-ctl -d "$CAMERA" -c keep_format=0
v4l2-ctl -d "$CAMERA" --set-fmt-video-out=width=1920,height=1080,pixelformat=YUYV
sudo v4l2loopback-ctl set-fps "$CAMERA" 30
v4l2-ctl -d "$CAMERA" -c keep_format=1
exec 3>&-
```

In a second terminal, set `CAMERA` to that same path and leave this bridge running:

```sh
CAMERA=/dev/video3  # replace with your path
gst-launch-1.0 -e \
  pipewiresrc target-object=tomori-spectator \
  ! 'video/x-raw,format=RGBA,width=1920,height=1080,framerate=30/1' \
  ! capssetter 'caps=video/x-raw,pixel-aspect-ratio=(fraction)1/1' \
  ! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 leaky=downstream \
  ! videoconvert \
  ! 'video/x-raw,format=YUY2,width=1920,height=1080,framerate=30/1,pixel-aspect-ratio=1/1' \
  ! filesink location="$CAMERA" sync=false
```

The verified bridge writes directly to the loopback device using `filesink`; `v4l2sink` produced buffer errors with the installed loopback driver. Once the bridge is writing, refresh the device's udev properties so applications see it as a capture device:

```sh
sudo udevadm trigger --action=change "/sys/devices/virtual/video4linux/${CAMERA##*/}"
udevadm settle
v4l2-ctl -d "$CAMERA" --all
udevadm info --query=property --name="$CAMERA" | grep ID_V4L_CAPABILITIES
```

The format should be 1920×1080 YUYV at 30 fps, with `Video Capture` and `ID_V4L_CAPABILITIES=:capture:`. Select **Tomori Spectator** as the camera in Slack. Slack may need restarting to discover a newly created device; do so only when it will not interrupt an active call.

## 3. Create and route a virtual microphone

Create an isolated sink and a microphone mapped to its monitor:

```sh
pactl load-module module-null-sink \
  sink_name=tomori_spectator_audio \
  sink_properties=device.description=Tomori-Spectator-Audio rate=48000 channels=2
pactl load-module module-remap-source \
  master=tomori_spectator_audio.monitor source_name=tomori_huddle_mic \
  source_properties=device.description=Tomori-Huddle-Microphone
```

Record the module IDs printed by these commands if you want to unload these exact modules later. Avoid creating duplicates if they already exist; inspect `pactl list short modules` and `pactl list short sources` first.

The simplest way to route the spectator's audio is to start it with the sink, once the sink exists, and skip the mapping below:

```sh
./target/debug/tomori --spectator-pipewire \
  --server 127.0.0.1:8765 --space SPACE_ID --audio-output tomori_spectator_audio
```

`--audio-output` chooses the output for that process only and changes no remembered route. To move a spectator that is already running instead, map it as follows.

Identify the spectator's process ID from the terminal that started it, or use `ps -eo pid,args` and select the `tomori --spectator-pipewire` process for the intended space. Multiple Tomori processes can have the same audio application name, so that name alone is not enough. Set `SPECTATOR_PID` to the chosen PID and map it to its PipeWire sink input and current sink:

```sh
export SPECTATOR_PID=1234  # replace with the spectator process ID
python3 - <<'PY'
import json, os, subprocess
pid = int(os.environ['SPECTATOR_PID'])
objects = json.loads(subprocess.check_output(['pw-dump']))
by_id = {obj['id']: obj for obj in objects}
inputs = {item['index']: item for item in json.loads(
    subprocess.check_output(['pactl', '-f', 'json', 'list', 'sink-inputs']))}
matches = []
for obj in objects:
    info = obj.get('info', {})
    props = info.get('props', {})
    if obj.get('type') != 'PipeWire:Interface:Node' or props.get('media.class') != 'Stream/Output/Audio':
        continue
    client = by_id.get(props.get('client.id'), {}).get('info', {}).get('props', {})
    if str(client.get('pipewire.sec.pid')) != str(pid):
        continue
    serial = props.get('object.serial')
    if serial in inputs:
        matches.append((serial, inputs[serial]['sink']))
if len(matches) != 1:
    raise SystemExit(f'Expected one spectator audio stream, found {len(matches)}; check ALSA/PipeWire routing.')
print('SINK_INPUT_ID=%s ORIGINAL_SINK_ID=%s' % matches[0])
PY
```

If the script finds no stream, ensure ALSA's default output uses the native PipeWire ALSA plugin and that the spectator has joined a server and opened its speaker; PipeWire's PulseAudio compatibility layer alone does not configure ALSA. When ALSA uses the PulseAudio plugin instead, this PID mapping may identify the PulseAudio server rather than the spectator and must not be used to guess an input. Check `pactl list short sinks` to identify the sink names and IDs. Copy the printed IDs below and route that input only. If its current sink is already `tomori_spectator_audio`, the old physical output cannot be inferred from this mapping; select a suitable existing output from the sink list if you need to restore it later.

```sh
SINK_INPUT_ID=1234    # replace with the printed sink input ID
ORIGINAL_SINK_ID=55   # replace with its printed original sink ID
pactl move-sink-input "$SINK_INPUT_ID" tomori_spectator_audio
pactl list short sink-inputs
```

This diverts the spectator's audio from the PC's normal headphones to the dedicated sink; it does not change the default output or Quest playback. **PipeWire can remember a route by application name.** The spectator modes name their audio **Tomori spectator** and the participant modes (`--desktop`, `--xr`, `--console`) **Tomori**, so a remembered route for the spectator does not send a participant's speaker here; builds before this change all used `PipeWire ALSA [tomori]`, and setting `PIPEWIRE_ALSA` yourself replaces the name. Another spectator may still be routed to this sink automatically, including one in a different space. Before selecting the microphone in Slack, and whenever a Tomori client starts or restarts, inspect `pactl list short sink-inputs` and use the PID mapping above to confirm that only the intended spectator targets `tomori_spectator_audio`. Do not share until unintended streams are moved elsewhere. Select **Tomori-Huddle-Microphone** in Slack and test its input meter while a participant or the agent speaks.

When done, stop sharing in Slack first. If the same spectator is still running, move its sink input back with `pactl move-sink-input "$SINK_INPUT_ID" "$ORIGINAL_SINK_ID"`; check the current PID mapping again first because input IDs can change after a restart. Then unload only the remap and null-sink module IDs recorded above, in that order. If another spectator starts later, check its output route because the application-name restore policy may remember the old destination.

## Troubleshooting

- If the camera is absent, confirm the bridge is running and the udev capability is `:capture:`. A loopback device with exclusive capabilities appears as output-only until a writer opens it; refreshing udev after the writer starts was necessary on the demo machine.
- If Slack sees the camera but the picture is blank, check the spectator log and the bridge terminal for errors. A listed PipeWire node alone does not prove frames are flowing. `v4l2-ctl -d "$CAMERA" --all` checks the negotiated format; preview the actual camera in Slack.
- If the microphone is silent, check that the spectator's sink input is routed to `tomori_spectator_audio`, that Slack selected `Tomori-Huddle-Microphone`, and that neither application has muted it. The microphone carries silence when nobody is speaking.
- The Quest's committed client currently sends microphone audio while X is held to address the agent. A separate live-audio toggle is still under review.
