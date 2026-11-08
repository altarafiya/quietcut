package io.github.altarafiya.quietcut;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.net.Uri;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import androidx.media3.common.MimeTypes;
import androidx.media3.common.util.UnstableApi;
import androidx.media3.effect.Presentation;
import androidx.media3.transformer.Composition;
import androidx.media3.transformer.DefaultEncoderFactory;
import androidx.media3.transformer.EditedMediaItem;
import androidx.media3.transformer.EditedMediaItemSequence;
import androidx.media3.transformer.Effects;
import androidx.media3.transformer.ExportException;
import androidx.media3.transformer.ExportResult;
import androidx.media3.transformer.ProgressHolder;
import androidx.media3.transformer.Transformer;
import androidx.media3.transformer.VideoEncoderSettings;
import com.google.common.collect.ImmutableList;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

@UnstableApi
final class ExportJob {
    interface Listener {
        void progress(String message, int percent);
        void finished(Uri uri, String description);
        void failed(String description);
    }

    record Options(boolean audioOnly, String mime, String encoder, double quality, boolean cutPauses) {}

    private final Context context;
    private final Executor worker;
    private final List<Clip> clips;
    private final Options options;
    private final Listener listener;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final AtomicBoolean cancelled = new AtomicBoolean();
    private final ProgressHolder progress = new ProgressHolder();
    private Transformer transformer;
    private Composition composition;
    private File temp;
    private boolean saving;
    private boolean softwareRetry;
    private int bitrate;
    private String encodingMessage;

    ExportJob(Context context, Executor worker, List<Clip> clips, Options options, Listener listener) {
        this.context = context.getApplicationContext();
        this.worker = worker;
        this.clips = new ArrayList<>(clips);
        this.options = options;
        this.listener = listener;
    }

    static List<MediaCodecInfo> encoders(String mime) {
        List<MediaCodecInfo> result = new ArrayList<>();
        for (MediaCodecInfo info : new MediaCodecList(MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
            if (!info.isEncoder() || info.isAlias()) continue;
            for (String type : info.getSupportedTypes()) {
                if (!mime.equalsIgnoreCase(type)) continue;
                try {
                    MediaCodecInfo.CodecCapabilities caps = info.getCapabilitiesForType(type);
                    for (int color : caps.colorFormats) {
                        if (color == MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface) {
                            result.add(info);
                            break;
                        }
                    }
                } catch (IllegalArgumentException ignored) {
                    // The advertised type does not have usable capabilities on this device.
                }
            }
        }
        result.sort((a, b) -> Boolean.compare(b.isHardwareAccelerated(), a.isHardwareAccelerated()));
        return result;
    }

    static String encoderDescription(MediaCodecInfo codec) {
        String kind = codec.isHardwareAccelerated() ? "Hardware" : codec.isSoftwareOnly() ? "Software" : "System";
        return kind + " · " + codec.getName();
    }

    void start() {
        listener.progress(options.cutPauses() ? "Finding quiet pauses…" : "Preparing the edit…", -1);
        worker.execute(() -> {
            try {
                List<PauseScanner.Span> spans = new ArrayList<>();
                for (Clip clip : clips) {
                    if (cancelled.get()) return;
                    if (options.cutPauses()) {
                        spans.addAll(PauseScanner.scan(context, clip, cancelled));
                    } else {
                        spans.add(new PauseScanner.Span(clip, clip.start(), clip.end()));
                    }
                    if (spans.size() > 2_000) throw new IOException("Too many cuts for one export. Try a shorter selection.");
                }
                if (cancelled.get()) return;
                composition = composition(spans);
                temp = File.createTempFile("quietcut-", ".mp4", context.getCacheDir());
                main.post(() -> {
                    if (cancelled.get()) deleteTemp();
                    else encode();
                });
            } catch (Exception e) {
                main.post(() -> fail(message(e)));
            }
        });
    }

    private Composition composition(List<PauseScanner.Span> spans) throws IOException {
        if (spans.isEmpty()) throw new IOException("No clips remain after cutting quiet pauses.");
        boolean hasAudio = false;
        for (PauseScanner.Span span : spans) hasAudio |= span.clip().audio();
        if (options.audioOnly() && !hasAudio) throw new IOException("This selection has no audio to export.");
        List<EditedMediaItem> items = new ArrayList<>();
        EditedMediaItemSequence.Builder sequence = new EditedMediaItemSequence.Builder(items);
        for (PauseScanner.Span span : spans) {
            Clip clip = span.clip();
            if (options.audioOnly() && !clip.audio()) {
                sequence.addGap((span.end() - span.start()) * 1000);
            } else {
                sequence.addItem(new EditedMediaItem.Builder(clip.item(span.start(), span.end()))
                        .setDurationUs(clip.duration() * 1000)
                        .setRemoveVideo(options.audioOnly())
                        .build());
            }
        }
        Composition.Builder builder = new Composition.Builder(sequence.build());
        if (hasAudio) builder.experimentalSetForceAudioTrack(true);
        if (!options.audioOnly()) {
            Clip first = spans.get(0).clip();
            double scale = Math.min(1, Math.min(1920.0 / Math.max(first.width(), first.height()),
                    1080.0 / Math.min(first.width(), first.height())));
            int width = Math.max(2, (int) (first.width() * scale) / 2 * 2);
            int height = Math.max(2, (int) (first.height() * scale) / 2 * 2);
            int fps = Math.max(15, Math.min(60, first.fps()));
            double factor = MimeTypes.VIDEO_H265.equals(options.mime()) ? 0.062 : 0.085;
            bitrate = (int) Math.max(200_000, width * height * fps * factor * options.quality());
            builder.setEffects(new Effects(Collections.emptyList(), Collections.singletonList(
                    Presentation.createForWidthAndHeight(width, height, Presentation.LAYOUT_SCALE_TO_FIT))));
            builder.setHdrMode(Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL);
        }
        return builder.build();
    }

    private void encode() {
        try {
            Transformer.Builder builder = new Transformer.Builder(context)
                    .setAudioMimeType(MimeTypes.AUDIO_AAC);
            boolean hardware = false;
            if (options.audioOnly()) {
                encodingMessage = "Encoding audio…";
            } else {
                List<MediaCodecInfo> candidates = encoders(options.mime());
                if (options.encoder() != null) {
                    candidates.removeIf(codec -> !codec.getName().equals(options.encoder()));
                } else if (softwareRetry) {
                    candidates.removeIf(MediaCodecInfo::isHardwareAccelerated);
                } else if (candidates.stream().anyMatch(MediaCodecInfo::isHardwareAccelerated)) {
                    candidates.removeIf(codec -> !codec.isHardwareAccelerated());
                }
                if (candidates.isEmpty()) throw new IOException("No compatible encoder is available for this format.");
                hardware = candidates.get(0).isHardwareAccelerated();
                encodingMessage = hardware ? "Encoding with hardware…" : "Encoding with the system codec…";
                ImmutableList<MediaCodecInfo> selected = ImmutableList.copyOf(candidates);
                builder.setVideoMimeType(options.mime())
                        .setEncoderFactory(new DefaultEncoderFactory.Builder(context)
                                .setVideoEncoderSelector(mime -> options.mime().equals(mime)
                                        ? selected : ImmutableList.of())
                                .setRequestedVideoEncoderSettings(new VideoEncoderSettings.Builder()
                                        .setBitrate(bitrate).build())
                                .build());
            }
            boolean canRetry = hardware && options.encoder() == null && !softwareRetry;
            builder.addListener(new Transformer.Listener() {
                @Override public void onCompleted(Composition input, ExportResult result) {
                    transformer = null;
                    main.removeCallbacks(progressUpdate);
                    if (!options.audioOnly() && (!options.mime().equals(result.videoMimeType)
                            || result.videoEncoderName == null)) {
                        fail("The selected video format or encoder could not be used.");
                        return;
                    }
                    if (options.encoder() != null && !options.audioOnly()
                            && !options.encoder().equals(result.videoEncoderName)) {
                        fail("Android could not use the selected encoder.");
                        return;
                    }
                    save(result);
                }

                @Override public void onError(Composition input, ExportResult result, ExportException error) {
                    transformer = null;
                    main.removeCallbacks(progressUpdate);
                    if (cancelled.get()) return;
                    if (canRetry && encoderError(error)
                            && encoders(options.mime()).stream().anyMatch(codec -> !codec.isHardwareAccelerated())) {
                        softwareRetry = true;
                        deleteTemp();
                        encode();
                    } else {
                        fail(encoderError(error) ? "The encoder could not process these clips. Try another available encoder or format."
                                : "Export failed: " + error.getErrorCodeName().replace("ERROR_CODE_", "").replace('_', ' ').toLowerCase(Locale.ROOT));
                    }
                }
            });
            transformer = builder.build();
            transformer.start(composition, temp.getAbsolutePath());
            listener.progress(encodingMessage, -1);
            main.post(progressUpdate);
        } catch (Exception error) {
            transformer = null;
            fail(message(error));
        }
    }

    private static boolean encoderError(ExportException error) {
        return error.errorCode == ExportException.ERROR_CODE_ENCODER_INIT_FAILED
                || error.errorCode == ExportException.ERROR_CODE_ENCODING_FAILED
                || error.errorCode == ExportException.ERROR_CODE_ENCODING_FORMAT_UNSUPPORTED;
    }

    private final Runnable progressUpdate = new Runnable() {
        @Override public void run() {
            if (transformer == null || cancelled.get()) return;
            int state = transformer.getProgress(progress);
            listener.progress(encodingMessage, state == Transformer.PROGRESS_STATE_AVAILABLE ? progress.progress : -1);
            main.postDelayed(this, 250);
        }
    };

    private void save(ExportResult result) {
        if (cancelled.get()) {
            deleteTemp();
            return;
        }
        saving = true;
        listener.progress("Saving to Downloads/QuietCut…", -1);
        worker.execute(() -> {
            Uri destination = null;
            ContentResolver resolver = context.getContentResolver();
            try {
                String stamp = new SimpleDateFormat("yyyyMMdd-HHmmss-SSS", Locale.ROOT).format(new Date());
                ContentValues values = new ContentValues();
                values.put(MediaStore.MediaColumns.DISPLAY_NAME,
                        "QuietCut-" + stamp + (options.audioOnly() ? ".m4a" : ".mp4"));
                values.put(MediaStore.MediaColumns.MIME_TYPE, options.audioOnly() ? "audio/mp4" : "video/mp4");
                values.put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/QuietCut");
                values.put(MediaStore.MediaColumns.IS_PENDING, 1);
                destination = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
                if (destination == null) throw new IOException("Could not create the output file.");
                try (FileInputStream input = new FileInputStream(temp);
                     OutputStream output = resolver.openOutputStream(destination, "w")) {
                    if (output == null) throw new IOException("Could not open the output file.");
                    byte[] buffer = new byte[128 * 1024];
                    int size;
                    while (!cancelled.get() && (size = input.read(buffer)) != -1) {
                        output.write(buffer, 0, size);
                    }
                }
                Uri finished = destination;
                main.post(() -> {
                    if (cancelled.get()) {
                        resolver.delete(finished, null, null);
                    } else {
                        ContentValues ready = new ContentValues();
                        ready.put(MediaStore.MediaColumns.IS_PENDING, 0);
                        try {
                            if (resolver.update(finished, ready, null, null) != 1) {
                                throw new IOException("Could not finish saving the output file.");
                            }
                            listener.finished(finished, describe(result));
                        } catch (Exception error) {
                            resolver.delete(finished, null, null);
                            fail(message(error));
                        }
                    }
                    deleteTemp();
                });
            } catch (Exception error) {
                if (destination != null) resolver.delete(destination, null, null);
                main.post(() -> fail(message(error)));
            }
        });
    }

    private String describe(ExportResult result) {
        if (options.audioOnly()) return "AAC audio · " + MainActivity.time(result.durationMs);
        for (MediaCodecInfo info : encoders(result.videoMimeType)) {
            if (info.getName().equals(result.videoEncoderName)) return encoderDescription(info);
        }
        return result.videoEncoderName;
    }

    void cancel() {
        cancelled.set(true);
        main.removeCallbacks(progressUpdate);
        if (transformer != null) {
            transformer.cancel();
            transformer = null;
        }
        if (!saving && temp != null) deleteTemp();
    }

    private void fail(String description) {
        deleteTemp();
        if (!cancelled.get()) listener.failed(description);
    }

    private void deleteTemp() {
        if (temp != null) temp.delete();
    }

    private static String message(Exception error) {
        String detail = error.getMessage();
        return detail == null || detail.trim().isEmpty() ? "Could not process the selected media." : detail;
    }
}
