package io.github.altarafiya.quietcut;

import android.content.Context;
import android.media.AudioFormat;
import android.media.MediaCodec;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

final class PauseScanner {
    record Span(Clip clip, long start, long end) {}

    private static final int WINDOW_MS = 20;
    private static final int MIN_PAUSE_MS = 500;
    private static final int EDGE_MS = 120;
    private static final double QUIET_POWER = Math.pow(10, -42.0 / 10);
    private static final long TIMEOUT_US = 10_000;

    static List<Span> scan(Context context, Clip clip, AtomicBoolean cancelled) throws IOException {
        if (!clip.audio()) return java.util.Collections.singletonList(new Span(clip, clip.start(), clip.end()));
        MediaExtractor extractor = new MediaExtractor();
        MediaCodec decoder = null;
        try {
            extractor.setDataSource(context, clip.uri(), null);
            MediaFormat audio = null;
            for (int i = 0; i < extractor.getTrackCount(); i++) {
                MediaFormat format = extractor.getTrackFormat(i);
                String mime = format.getString(MediaFormat.KEY_MIME);
                if (mime != null && mime.startsWith("audio/")) {
                    audio = format;
                    extractor.selectTrack(i);
                    break;
                }
            }
            if (audio == null) return java.util.Collections.singletonList(new Span(clip, clip.start(), clip.end()));
            decoder = MediaCodec.createDecoderByType(audio.getString(MediaFormat.KEY_MIME));
            decoder.configure(audio, null, null, 0);
            decoder.start();
            extractor.seekTo(clip.start() * 1000, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
            long duration = clip.end() - clip.start();
            int windows = Math.toIntExact((duration + WINDOW_MS - 1) / WINDOW_MS);
            double[] power = new double[windows];
            int[] counts = new int[windows];
            decode(extractor, decoder, audio, clip, power, counts, cancelled);
            return keptSpans(clip, power, counts);
        } finally {
            if (decoder != null) decoder.release();
            extractor.release();
        }
    }

    private static void decode(MediaExtractor source, MediaCodec decoder, MediaFormat audio,
                               Clip clip, double[] power, int[] counts, AtomicBoolean cancelled)
            throws IOException {
        int channels = audio.getInteger(MediaFormat.KEY_CHANNEL_COUNT);
        int rate = audio.getInteger(MediaFormat.KEY_SAMPLE_RATE);
        int encoding = AudioFormat.ENCODING_PCM_16BIT;
        boolean inputEnded = false;
        boolean decoded = false;
        MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
        long lastProgress = android.os.SystemClock.elapsedRealtime();
        while (!cancelled.get()) {
            if (!inputEnded) {
                int input = decoder.dequeueInputBuffer(TIMEOUT_US);
                if (input >= 0) {
                    ByteBuffer buffer = decoder.getInputBuffer(input);
                    long time = source.getSampleTime();
                    int size = time < 0 || time >= clip.end() * 1000
                            ? -1 : source.readSampleData(buffer, 0);
                    inputEnded = size < 0;
                    decoder.queueInputBuffer(input, 0, Math.max(0, size), Math.max(0, time),
                            inputEnded ? MediaCodec.BUFFER_FLAG_END_OF_STREAM : 0);
                    if (!inputEnded) source.advance();
                    lastProgress = android.os.SystemClock.elapsedRealtime();
                }
            }
            int output = decoder.dequeueOutputBuffer(info, TIMEOUT_US);
            if (output == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                MediaFormat pcm = decoder.getOutputFormat();
                rate = pcm.getInteger(MediaFormat.KEY_SAMPLE_RATE);
                channels = pcm.getInteger(MediaFormat.KEY_CHANNEL_COUNT);
                if (pcm.containsKey(MediaFormat.KEY_PCM_ENCODING)) {
                    encoding = pcm.getInteger(MediaFormat.KEY_PCM_ENCODING);
                }
                if (encoding != AudioFormat.ENCODING_PCM_16BIT && encoding != AudioFormat.ENCODING_PCM_FLOAT) {
                    throw new IOException("Unsupported audio format for pause detection.");
                }
            } else if (output >= 0) {
                ByteBuffer buffer = decoder.getOutputBuffer(output);
                if (buffer != null && info.size > 0) {
                    buffer.position(info.offset).limit(info.offset + info.size);
                    buffer.order(ByteOrder.nativeOrder());
                    accumulate(buffer, info.presentationTimeUs, clip.start() * 1000,
                            rate, channels, encoding, power, counts);
                    decoded = true;
                }
                decoder.releaseOutputBuffer(output, false);
                lastProgress = android.os.SystemClock.elapsedRealtime();
                if ((info.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                    if (!decoded) throw new IOException("No audio could be decoded in this selection.");
                    return;
                }
            }
            if (android.os.SystemClock.elapsedRealtime() - lastProgress > 10_000) {
                throw new IOException("The audio decoder stopped responding.");
            }
        }
    }

    private static void accumulate(ByteBuffer buffer, long pts, long startUs, int rate, int channels,
                                   int encoding, double[] power, int[] counts) {
        boolean floats = encoding == AudioFormat.ENCODING_PCM_FLOAT;
        int frameBytes = (floats ? Float.BYTES : Short.BYTES) * channels;
        long first = Math.round((pts - startUs) * rate / 1_000_000.0);
        for (long frame = first; buffer.remaining() >= frameBytes; frame++) {
            double peak = 0;
            for (int channel = 0; channel < channels; channel++) {
                double sample = floats ? buffer.getFloat() : buffer.getShort() / 32768.0;
                peak = Math.max(peak, sample * sample);
            }
            if (frame < 0) continue;
            long bin = frame * 1000 / ((long) rate * WINDOW_MS);
            if (bin >= power.length) return;
            power[(int) bin] += peak;
            counts[(int) bin]++;
        }
    }

    private static List<Span> keptSpans(Clip clip, double[] power, int[] counts) {
        List<Span> spans = new ArrayList<>();
        long cursor = clip.start();
        int quietStart = -1;
        for (int i = 0; i <= power.length; i++) {
            boolean quiet = i < power.length && (counts[i] == 0 || power[i] / counts[i] < QUIET_POWER);
            if (quiet) {
                if (quietStart < 0) quietStart = i;
                continue;
            }
            if (quietStart < 0) continue;
            long start = clip.start() + (long) quietStart * WINDOW_MS;
            long end = Math.min(clip.end(), clip.start() + (long) i * WINDOW_MS);
            if (end - start >= MIN_PAUSE_MS) {
                long cutStart = start == clip.start() ? start : start + EDGE_MS;
                long cutEnd = end == clip.end() ? end : end - EDGE_MS;
                if (cutStart > cursor) spans.add(new Span(clip, cursor, cutStart));
                cursor = cutEnd;
            }
            quietStart = -1;
        }
        if (cursor < clip.end()) spans.add(new Span(clip, cursor, clip.end()));
        return spans;
    }
}
