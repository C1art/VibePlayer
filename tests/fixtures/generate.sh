#!/usr/bin/env bash
# Regenerates the test fixtures. Needs an ffmpeg CLI with libx264, libx265 and libopus.
set -euo pipefail
cd "$(dirname "$0")"

ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }
video="testsrc2=size=320x180:rate=25"
tone="sine=frequency=440:sample_rate=48000"

cat > subtitles.srt <<'SRT'
1
00:00:00,500 --> 00:00:01,500
First line

2
00:00:02,000 --> 00:00:03,000
<i>Second</i> line
SRT

# H.264 with B-frames, closed GOP, AAC; the general-purpose fixture.
ff -f lavfi -i "$video:duration=4" -f lavfi -i "$tone:duration=4" \
  -c:v libx264 -preset veryfast -crf 30 -bf 3 -g 25 -keyint_min 25 -sc_threshold 0 -pix_fmt yuv420p \
  -c:a aac -b:a 64k -movflags +faststart h264_bframes.mp4

# HEVC with open GOP and Opus.
ff -f lavfi -i "$video:duration=3" -f lavfi -i "$tone:duration=3" \
  -c:v libx265 -preset ultrafast -crf 32 -x265-params "open-gop=1:keyint=30:min-keyint=30:bframes=3:log-level=error" \
  -c:a libopus -b:a 48k hevc_open_gop.mkv

# Variable frame rate: the first second at 25 fps, then every third frame.
ff -f lavfi -i "$video:duration=3" \
  -vf "select='lt(n\,25)+not(mod(n\,3))'" -fps_mode vfr \
  -c:v libx264 -preset veryfast -crf 30 -pix_fmt yuv420p vfr.mkv

# Interlaced MPEG-2 in MPEG-TS with MP2 audio.
ff -f lavfi -i "testsrc2=size=320x240:rate=25:duration=2" -f lavfi -i "$tone:duration=2" \
  -c:v mpeg2video -b:v 600k -flags +ilme+ildct -top 1 -c:a mp2 -b:a 96k interlaced.ts

# Two audio tracks with languages and an embedded SubRip track.
ff -f lavfi -i "$video:duration=4" -f lavfi -i "$tone:duration=4" -f lavfi -i "sine=frequency=880:sample_rate=44100:duration=4" \
  -i subtitles.srt -map 0 -map 1 -map 2 -map 3 \
  -c:v libx264 -preset veryfast -crf 32 -pix_fmt yuv420p -c:a:0 aac -b:a:0 64k -c:a:1 ac3 -b:a:1 96k -c:s srt \
  -metadata:s:a:0 language=eng -metadata:s:a:1 language=ger -metadata:s:s:0 language=eng \
  -metadata title="Multi track" multi_audio.mkv

# HDR10: 10-bit HEVC, BT.2020 primaries, PQ transfer, mastering metadata.
ff -f lavfi -i "$video:duration=2" \
  -vf "format=yuv420p10le" -c:v libx265 -preset ultrafast -crf 32 \
  -x265-params "colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:range=limited:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400:hdr10=1:log-level=error" \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc hdr10.mkv

# Truncated: the general fixture cut off partway through its media data.
head -c 40000 h264_bframes.mp4 > truncated.mp4

rm subtitles.srt
ls -l
