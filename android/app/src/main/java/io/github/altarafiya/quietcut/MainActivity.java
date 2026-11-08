package io.github.altarafiya.quietcut;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.res.ColorStateList;
import android.database.Cursor;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.media.MediaCodecInfo;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Spinner;
import android.widget.Switch;
import android.widget.TextView;
import androidx.media3.common.MimeTypes;
import androidx.media3.common.util.UnstableApi;
import androidx.media3.exoplayer.ExoPlayer;
import androidx.media3.ui.PlayerView;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONArray;
import org.json.JSONException;

@UnstableApi
public final class MainActivity extends Activity {
    private static final int PICK_VIDEOS = 10;
    private static final int BG = 0xFF111019, SURFACE = 0xFF1C1A28, FIELD = 0xFF27243A;
    private static final int TEXT = 0xFFF5F1FF, MUTED = 0xFFBFB9D0, ACCENT = 0xFFD4C5FF;
    private static final double[] BITRATE_MULTIPLIERS = {0.63, 1.0, 1.52};

    private final ArrayList<Clip> clips = new ArrayList<>();
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private final ArrayList<String> formats = new ArrayList<>();
    private final ArrayList<String> formatLabels = new ArrayList<>();
    private final ArrayList<MediaCodecInfo> codecs = new ArrayList<>();
    private SharedPreferences preferences;
    private ExoPlayer player;
    private PlayerView playerView;
    private FrameLayout preview;
    private TextView previewEmpty, editHeading, timelineTime, encoderHint, status;
    private LinearLayout timeline, trimBox, videoSettings;
    private Button add, export, cancel, open;
    private Spinner format, encoder, quality;
    private Switch cutPauses;
    private ProgressBar progress;
    private ExportJob job;
    private Uri lastOutput;
    private String lastMime;
    private int selected = -1;
    private boolean importing;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        preferences = getSharedPreferences("project", MODE_PRIVATE);
        restore();
        player = new ExoPlayer.Builder(this).build();
        makeScreen();
        refreshTimeline();
        loadPreview();
    }

    private void makeScreen() {
        LinearLayout page = column();
        page.setBackgroundColor(BG);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        setContentView(page);
        if (Build.VERSION.SDK_INT >= 30) {
            page.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
                return insets;
            });
        }
        LinearLayout header = row();
        header.setGravity(Gravity.CENTER_VERTICAL);
        header.setPadding(dp(18), dp(10), dp(18), dp(9));
        ImageView icon = new ImageView(this);
        icon.setImageResource(io.github.altarafiya.quietcut.R.drawable.ic_quietcut);
        icon.setBackground(shape(FIELD, 11));
        icon.setClipToOutline(true);
        header.addView(icon, new LinearLayout.LayoutParams(dp(42), dp(42)));
        LinearLayout heading = column();
        heading.setPadding(dp(11), 0, 0, 0);
        heading.addView(text("QuietCut", 23, TEXT, true));
        heading.addView(text("Video & audio editor", 12, MUTED, false));
        header.addView(heading, new LinearLayout.LayoutParams(0, -2, 1));
        add = button("Add videos", false, this::pick);
        header.addView(add);
        page.addView(header);

        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setClipToPadding(false);
        LinearLayout content = column();
        content.setPadding(dp(16), dp(4), dp(16), dp(14));
        scroll.addView(content, new ScrollView.LayoutParams(-1, -2));
        page.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));

        preview = new FrameLayout(this);
        preview.setBackground(shape(SURFACE, 14));
        preview.setClipToOutline(true);
        playerView = new PlayerView(this);
        playerView.setPlayer(player);
        playerView.setShowNextButton(false);
        playerView.setShowPreviousButton(false);
        playerView.setControllerShowTimeoutMs(2800);
        preview.addView(playerView, new FrameLayout.LayoutParams(-1, -1));
        previewEmpty = text("Add videos to begin", 17, MUTED, true);
        previewEmpty.setGravity(Gravity.CENTER);
        preview.addView(previewEmpty, new FrameLayout.LayoutParams(-1, -1));
        addBlock(content, preview, 211, 12);

        LinearLayout projectCard = card();
        LinearLayout projectHeader = row();
        projectHeader.setGravity(Gravity.CENTER_VERTICAL);
        projectHeader.addView(text("Timeline", 17, TEXT, true), new LinearLayout.LayoutParams(0, -2, 1));
        timelineTime = text("", 13, ACCENT, true);
        projectHeader.addView(timelineTime);
        projectCard.addView(projectHeader);
        timeline = column();
        addBlock(projectCard, timeline, -2, 8);
        trimBox = column();
        addBlock(projectCard, trimBox, -2, 0);
        addBlock(content, projectCard, -2, 12);

        LinearLayout exportCard = card();
        exportCard.addView(text("Export settings", 17, TEXT, true));
        formats.add(MimeTypes.VIDEO_H264);
        formatLabels.add("MP4 · H.264");
        if (!ExportJob.encoders(MimeTypes.VIDEO_H265).isEmpty()) {
            formats.add(MimeTypes.VIDEO_H265);
            formatLabels.add("MP4 · HEVC");
        }
        formats.add(MimeTypes.AUDIO_AAC);
        formatLabels.add("M4A · Audio only");
        format = spinner(formatLabels);
        addField(exportCard, "Format", format);
        videoSettings = column();
        encoder = spinner(Collections.singletonList("Auto"));
        addField(videoSettings, "Video encoder", encoder);
        encoderHint = text("", 12, MUTED, false);
        addBlock(videoSettings, encoderHint, -2, 0);
        quality = spinner(java.util.Arrays.asList("Smaller file", "Balanced", "More detail"));
        quality.setSelection(1);
        addField(videoSettings, "Quality", quality);
        exportCard.addView(videoSettings);
        cutPauses = new Switch(this);
        cutPauses.setText("Remove quiet pauses");
        cutPauses.setTextColor(TEXT);
        cutPauses.setMinHeight(dp(42));
        cutPauses.setChecked(preferences.getBoolean("cutPauses", false));
        cutPauses.setOnCheckedChangeListener((button, checked) ->
                preferences.edit().putBoolean("cutPauses", checked).apply());
        addBlock(exportCard, cutPauses, -2, 4);
        addBlock(exportCard, text("Cuts quiet stretches of at least half a second.", 12, MUTED, false), -2, 1);
        addBlock(content, exportCard, -2, 0);

        LinearLayout footer = column();
        footer.setBackgroundColor(SURFACE);
        footer.setPadding(dp(16), dp(10), dp(16), dp(10));
        status = text("Exports save to Downloads/QuietCut.", 12, MUTED, false);
        footer.addView(status);
        progress = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progress.setMax(100);
        progress.setProgressTintList(ColorStateList.valueOf(ACCENT));
        progress.setVisibility(View.GONE);
        addBlock(footer, progress, 5, 4);
        LinearLayout actions = row();
        actions.setGravity(Gravity.CENTER_VERTICAL);
        export = button("Export video", true, this::startExport);
        actions.addView(export, new LinearLayout.LayoutParams(0, dp(46), 1));
        cancel = button("Cancel", false, this::cancelExport);
        open = button("Open file", false, this::openOutput);
        LinearLayout.LayoutParams next = new LinearLayout.LayoutParams(-2, dp(46));
        next.leftMargin = dp(8);
        actions.addView(cancel, next);
        LinearLayout.LayoutParams last = new LinearLayout.LayoutParams(-2, dp(46));
        last.leftMargin = dp(8);
        actions.addView(open, last);
        addBlock(footer, actions, -2, 0);
        page.addView(footer);
        format.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, View view, int position, long id) {
                updateFormat();
            }

            @Override public void onNothingSelected(AdapterView<?> parent) {}
        });
        updateFormat();
    }

    private void updateFormat() {
        boolean audio = audioOnly();
        videoSettings.setVisibility(audio ? View.GONE : View.VISIBLE);
        codecs.clear();
        if (!audio) {
            codecs.addAll(ExportJob.encoders(currentMime()));
            ArrayList<String> choices = new ArrayList<>();
            choices.add("Auto · hardware first");
            for (MediaCodecInfo codec : codecs) choices.add(ExportJob.encoderDescription(codec));
            encoder.setAdapter(adapter(choices));
            long hardware = codecs.stream().filter(MediaCodecInfo::isHardwareAccelerated).count();
            encoderHint.setText(codecs.isEmpty() ? "No compatible video encoder found."
                    : hardware == 0 ? "This device reports software video encoding for this format."
                    : hardware + (hardware == 1 ? " hardware encoder available." : " hardware encoders available."));
        }
        export.setText(audio ? "Export audio" : "Export video");
        refreshControls();
    }

    private String currentMime() {
        return formats.get(Math.max(0, format.getSelectedItemPosition()));
    }

    private boolean audioOnly() {
        return MimeTypes.AUDIO_AAC.equals(currentMime());
    }

    private void pick() {
        Intent picker = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("video/*")
                .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        try {
            startActivityForResult(picker, PICK_VIDEOS);
        } catch (ActivityNotFoundException error) {
            status.setText("No file picker is available on this device.");
        }
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_VIDEOS || result != RESULT_OK || data == null) return;
        ArrayList<Uri> uris = new ArrayList<>();
        ClipData picked = data.getClipData();
        if (picked != null) {
            for (int i = 0; i < picked.getItemCount(); i++) uris.add(picked.getItemAt(i).getUri());
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        if (uris.isEmpty()) return;
        importing = true;
        status.setText("Reading videos…");
        refreshControls();
        worker.execute(() -> {
            ArrayList<Clip> added = new ArrayList<>();
            int errors = 0;
            for (Uri uri : uris) {
                try {
                    Clip clip = Clip.inspect(this, uri, fileName(uri));
                    try {
                        getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
                    } catch (SecurityException ignored) {
                        // Some providers grant access for this session only.
                    }
                    added.add(clip);
                } catch (Exception error) {
                    errors++;
                }
            }
            int rejected = errors;
            runOnUiThread(() -> {
                if (isDestroyed()) return;
                importing = false;
                if (!added.isEmpty() && selected < 0) selected = clips.size();
                clips.addAll(added);
                persist();
                refreshTimeline();
                loadPreview();
                status.setText(rejected == 0 ? "Videos ready to edit."
                        : rejected + (rejected == 1 ? " video could not be read." : " videos could not be read."));
            });
        });
    }

    private String fileName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME},
                null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && cursor.getString(0) != null) return cursor.getString(0);
        }
        return "Video";
    }

    private void refreshTimeline() {
        timeline.removeAllViews();
        long duration = 0;
        for (Clip clip : clips) duration += clip.end() - clip.start();
        timelineTime.setText(time(duration));
        if (clips.isEmpty()) addBlock(timeline, text("Add one or more clips.", 13, MUTED, false), -2, 4);
        for (int i = 0; i < clips.size(); i++) {
            int index = i;
            Clip clip = clips.get(i);
            LinearLayout item = row();
            item.setPadding(dp(10), dp(8), dp(10), dp(8));
            item.setGravity(Gravity.CENTER_VERTICAL);
            item.setBackground(touch(i == selected ? 0xFF39304D : FIELD, 9));
            item.setOnClickListener(view -> {
                if (job != null || importing) return;
                selected = index;
                refreshTimeline();
                loadPreview();
            });
            item.setFocusable(true);
            TextView number = text(String.format(Locale.ROOT, "%02d", i + 1), 13, ACCENT, true);
            item.addView(number, new LinearLayout.LayoutParams(dp(33), -2));
            TextView name = text(clip.name(), 14, TEXT, i == selected);
            name.setSingleLine();
            name.setEllipsize(TextUtils.TruncateAt.END);
            item.addView(name, new LinearLayout.LayoutParams(0, -2, 1));
            TextView time = text(time(clip.end() - clip.start()), 12, MUTED, false);
            time.setPadding(dp(9), 0, 0, 0);
            item.addView(time);
            addBlock(timeline, item, -2, 5);
        }
        trimBox.removeAllViews();
        if (selected >= 0 && selected < clips.size()) makeTrim();
        refreshControls();
    }

    private void makeTrim() {
        Clip clip = clips.get(selected);
        editHeading = text("Trim clip " + (selected + 1), 14, TEXT, true);
        addBlock(trimBox, editHeading, -2, 12);
        long[] bounds = {clip.start(), clip.end()};
        TextView in = text("", 12, MUTED, false);
        TextView out = text("", 12, MUTED, false);
        Runnable labels = () -> {
            in.setText("In  " + time(bounds[0]));
            out.setText("Out  " + time(bounds[1]));
        };
        labels.run();
        trimBox.addView(in);
        trimBox.addView(trimSlider(clip, bounds, 0, labels));
        trimBox.addView(out);
        trimBox.addView(trimSlider(clip, bounds, 1, labels));
        LinearLayout moves = row();
        Button up = button("Move up", false, () -> move(-1));
        up.setEnabled(selected > 0);
        moves.addView(up, new LinearLayout.LayoutParams(0, dp(42), 1));
        Button down = button("Move down", false, () -> move(1));
        down.setEnabled(selected < clips.size() - 1);
        LinearLayout.LayoutParams middle = new LinearLayout.LayoutParams(0, dp(42), 1);
        middle.leftMargin = dp(5);
        moves.addView(down, middle);
        LinearLayout.LayoutParams end = new LinearLayout.LayoutParams(0, dp(42), 1);
        end.leftMargin = dp(5);
        moves.addView(button("Remove", false, this::remove), end);
        addBlock(trimBox, moves, -2, 2);
    }

    private SeekBar trimSlider(Clip clip, long[] bounds, int side, Runnable labels) {
        SeekBar slider = new SeekBar(this);
        slider.setMax((int) clip.duration());
        slider.setProgress((int) bounds[side]);
        slider.setContentDescription(side == 0 ? "Trim start" : "Trim end");
        slider.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override public void onProgressChanged(SeekBar bar, int value, boolean fromUser) {
                if (!fromUser) return;
                long minimum = Math.min(100, clip.duration());
                bounds[side] = side == 0 ? Math.min(value, bounds[1] - minimum)
                        : Math.max(value, bounds[0] + minimum);
                bar.setProgress((int) bounds[side]);
                labels.run();
                clips.set(selected, clip.trim(bounds[0], bounds[1]));
                persist();
            }

            @Override public void onStartTrackingTouch(SeekBar bar) {
                player.pause();
            }

            @Override public void onStopTrackingTouch(SeekBar bar) {
                refreshTimeline();
                loadPreview();
            }
        });
        return slider;
    }

    private void move(int direction) {
        int target = selected + direction;
        if (target < 0 || target >= clips.size()) return;
        Collections.swap(clips, selected, target);
        selected = target;
        persist();
        refreshTimeline();
    }

    private void remove() {
        clips.remove(selected);
        selected = Math.min(selected, clips.size() - 1);
        persist();
        refreshTimeline();
        loadPreview();
    }

    private void loadPreview() {
        player.stop();
        boolean available = selected >= 0 && selected < clips.size();
        previewEmpty.setVisibility(available ? View.GONE : View.VISIBLE);
        playerView.setVisibility(available ? View.VISIBLE : View.GONE);
        if (!available) {
            player.clearMediaItems();
            return;
        }
        Clip clip = clips.get(selected);
        player.setMediaItem(clip.item(clip.start(), clip.end()));
        player.setPlayWhenReady(false);
        player.prepare();
    }

    private void startExport() {
        if (job != null || clips.isEmpty()) return;
        player.stop();
        String mime = currentMime();
        int choice = encoder.getSelectedItemPosition();
        String codec = !audioOnly() && choice > 0 ? codecs.get(choice - 1).getName() : null;
        ExportJob.Options options = new ExportJob.Options(audioOnly(), mime, codec,
                BITRATE_MULTIPLIERS[Math.max(0, quality.getSelectedItemPosition())], cutPauses.isChecked());
        lastOutput = null;
        job = new ExportJob(this, worker, clips, options, new ExportJob.Listener() {
            @Override public void progress(String label, int percent) {
                status.setText(percent < 0 ? label : label + " " + percent + "%");
                progress.setIndeterminate(percent < 0);
                if (percent >= 0) progress.setProgress(percent);
            }

            @Override public void finished(Uri uri, String description) {
                lastOutput = uri;
                lastMime = options.audioOnly() ? "audio/mp4" : "video/mp4";
                finishExport("Saved to Downloads/QuietCut\n" + description);
            }

            @Override public void failed(String description) {
                finishExport(description);
                new AlertDialog.Builder(MainActivity.this).setTitle("Could not export")
                        .setMessage(description).setPositiveButton("OK", null).show();
            }
        });
        refreshControls();
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        job.start();
    }

    private void cancelExport() {
        if (job == null) return;
        job.cancel();
        finishExport("Export cancelled.");
    }

    private void finishExport(String message) {
        job = null;
        status.setText(message);
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        refreshControls();
        loadPreview();
    }

    private void openOutput() {
        if (lastOutput == null) return;
        Intent view = new Intent(Intent.ACTION_VIEW).setDataAndType(lastOutput, lastMime)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivity(view);
        } catch (ActivityNotFoundException error) {
            status.setText("Find the exported file in Downloads/QuietCut.");
        }
    }

    private void refreshControls() {
        boolean idle = job == null && !importing;
        add.setEnabled(idle);
        format.setEnabled(idle);
        quality.setEnabled(idle);
        encoder.setEnabled(idle);
        cutPauses.setEnabled(idle);
        setEnabled(trimBox, idle);
        if (idle && trimBox.getChildCount() > 0) {
            LinearLayout moves = (LinearLayout) trimBox.getChildAt(trimBox.getChildCount() - 1);
            moves.getChildAt(0).setEnabled(selected > 0);
            moves.getChildAt(1).setEnabled(selected < clips.size() - 1);
        }
        export.setEnabled(idle && !clips.isEmpty() && (audioOnly() || !codecs.isEmpty()));
        cancel.setVisibility(job == null ? View.GONE : View.VISIBLE);
        open.setVisibility(job == null && lastOutput != null ? View.VISIBLE : View.GONE);
        progress.setVisibility(job == null ? View.GONE : View.VISIBLE);
    }

    private void setEnabled(LinearLayout group, boolean enabled) {
        for (int i = 0; i < group.getChildCount(); i++) {
            View view = group.getChildAt(i);
            view.setEnabled(enabled);
            if (view instanceof LinearLayout child) setEnabled(child, enabled);
        }
    }

    private void persist() {
        JSONArray data = new JSONArray();
        try {
            for (Clip clip : clips) data.put(clip.json());
            preferences.edit().putString("clips", data.toString()).apply();
        } catch (JSONException error) {
            status.setText("Could not save the project.");
        }
    }

    private void restore() {
        try {
            JSONArray data = new JSONArray(preferences.getString("clips", "[]"));
            for (int i = 0; i < data.length(); i++) clips.add(Clip.fromJson(data.getJSONObject(i)));
        } catch (JSONException error) {
            clips.clear();
        }
        if (!clips.isEmpty()) selected = 0;
    }

    @Override protected void onStop() {
        player.pause();
        super.onStop();
    }

    @Override protected void onDestroy() {
        if (job != null) job.cancel();
        playerView.setPlayer(null);
        player.release();
        worker.shutdown();
        super.onDestroy();
    }

    static String time(long ms) {
        long seconds = Math.max(0, ms / 1000);
        return String.format(Locale.ROOT, "%d:%02d.%d", seconds / 60, seconds % 60, Math.max(0, ms / 100) % 10);
    }

    private LinearLayout card() {
        LinearLayout card = column();
        card.setPadding(dp(12), dp(11), dp(12), dp(11));
        card.setBackground(shape(SURFACE, 13));
        return card;
    }

    private LinearLayout row() {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        return row;
    }

    private LinearLayout column() {
        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        return column;
    }

    private void addBlock(LinearLayout layout, View view, int height, int top) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, height < 0 ? height : dp(height));
        params.topMargin = dp(top);
        layout.addView(view, params);
    }

    private void addField(LinearLayout layout, String label, View view) {
        TextView field = text(label, 12, MUTED, true);
        view.setContentDescription(label);
        addBlock(layout, field, -2, 8);
        addBlock(layout, view, 42, 3);
    }

    private Spinner spinner(List<String> items) {
        Spinner spinner = new Spinner(this, Spinner.MODE_DROPDOWN);
        spinner.setPadding(dp(10), 0, dp(10), 0);
        spinner.setBackground(touch(FIELD, 8));
        spinner.setAdapter(adapter(items));
        return spinner;
    }

    private ArrayAdapter<String> adapter(List<String> items) {
        return new ArrayAdapter<>(this, android.R.layout.simple_spinner_item, items) {
            @Override public View getView(int position, View recycled, android.view.ViewGroup parent) {
                TextView view = text(getItem(position) + "  ▾", 14, TEXT, false);
                view.setSingleLine();
                view.setEllipsize(TextUtils.TruncateAt.END);
                view.setGravity(Gravity.CENTER_VERTICAL);
                return view;
            }

            @Override public View getDropDownView(int position, View recycled, android.view.ViewGroup parent) {
                TextView view = text(getItem(position), 14, TEXT, false);
                view.setPadding(dp(12), dp(12), dp(12), dp(12));
                view.setBackground(touch(SURFACE, 0));
                return view;
            }
        };
    }

    private TextView text(String content, int size, int color, boolean bold) {
        TextView view = new TextView(this);
        view.setText(content);
        view.setTextSize(size);
        view.setTextColor(color);
        if (bold) view.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        return view;
    }

    private Button button(String label, boolean primary, Runnable action) {
        Button button = new Button(this);
        button.setAllCaps(false);
        button.setText(label);
        button.setTextSize(13);
        button.setTextColor(ColorStateList.valueOf(primary ? BG : TEXT));
        button.setPadding(dp(9), 0, dp(9), 0);
        button.setMinWidth(dp(58));
        button.setMinHeight(dp(41));
        button.setBackground(touch(primary ? ACCENT : FIELD, 9));
        button.setOnClickListener(view -> action.run());
        return button;
    }

    private GradientDrawable shape(int color, int radius) {
        GradientDrawable background = new GradientDrawable();
        background.setColor(color);
        background.setCornerRadius(dp(radius));
        return background;
    }

    private RippleDrawable touch(int color, int radius) {
        return new RippleDrawable(ColorStateList.valueOf(0x28FFFFFF), shape(color, radius), shape(Color.WHITE, radius));
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }
}
