package io.github.altarafiya.quietcut;

import android.content.Context;
import android.media.MediaMetadataRetriever;
import android.net.Uri;
import androidx.media3.common.MediaItem;
import java.io.IOException;
import org.json.JSONException;
import org.json.JSONObject;

record Clip(Uri uri, String name, long duration, long start, long end, int width, int height,
            int fps, boolean audio) {
    Clip trim(long from, long to) {
        return new Clip(uri, name, duration, from, to, width, height, fps, audio);
    }

    MediaItem item(long from, long to) {
        return new MediaItem.Builder()
                .setUri(uri)
                .setClippingConfiguration(new MediaItem.ClippingConfiguration.Builder()
                        .setStartPositionMs(from)
                        .setEndPositionMs(to)
                        .build())
                .build();
    }

    static Clip inspect(Context context, Uri uri, String name) throws IOException {
        try (MediaMetadataRetriever reader = new MediaMetadataRetriever()) {
            reader.setDataSource(context, uri);
            long duration = integer(reader, MediaMetadataRetriever.METADATA_KEY_DURATION, 0);
            int width = integer(reader, MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH, 0);
            int height = integer(reader, MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT, 0);
            int rotation = integer(reader, MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION, 0);
            int frames = integer(reader, MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT, 0);
            if (duration <= 0 || width <= 0 || height <= 0) {
                throw new IOException("Could not read this video.");
            }
            int fps = frames > 0 ? (int) Math.round(frames * 1000.0 / duration) : 30;
            boolean audio = "yes".equals(reader.extractMetadata(MediaMetadataRetriever.METADATA_KEY_HAS_AUDIO));
            boolean sideways = rotation == 90 || rotation == 270;
            return new Clip(uri, name, duration, 0, duration, sideways ? height : width,
                    sideways ? width : height, Math.max(1, fps), audio);
        }
    }

    private static int integer(MediaMetadataRetriever reader, int key, int fallback) {
        try {
            return Integer.parseInt(reader.extractMetadata(key));
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    JSONObject json() throws JSONException {
        return new JSONObject().put("uri", uri.toString()).put("name", name)
                .put("duration", duration).put("start", start).put("end", end)
                .put("width", width).put("height", height).put("fps", fps).put("audio", audio);
    }

    static Clip fromJson(JSONObject data) throws JSONException {
        return new Clip(Uri.parse(data.getString("uri")), data.getString("name"),
                data.getLong("duration"), data.getLong("start"), data.getLong("end"),
                data.getInt("width"), data.getInt("height"), data.getInt("fps"), data.getBoolean("audio"));
    }
}
