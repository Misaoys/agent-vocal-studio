using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text.Encodings.Web;
using System.Text.Json;
using NAudio.Wave;
using OpenUtau.Classic;
using OpenUtau.Core;
using OpenUtau.Core.Format;
using OpenUtau.Core.Ustx;

internal static class Program {
    private static int Main(string[] args) {
        try {
            Console.OutputEncoding = System.Text.Encoding.UTF8;
            if (args is not ["catalog"] and not ["render", _, _]) {
                throw new ArgumentException("Usage: Studio.OpenUtau catalog | render input.ustx output.wav");
            }
            System.Text.Encoding.RegisterProvider(System.Text.CodePagesEncodingProvider.Instance);
            var callbacks = new BlockingCollection<Action>();
            var doc = DocManager.Inst;
            doc.Initialize(Thread.CurrentThread, new RenderScheduler(callbacks));
            doc.PostOnUIThread = callbacks.Add;
            ToolsManager.Inst.Initialize();
            SingerManager.Inst.Initialize();
            if (args[0] == "catalog") {
                var voices = SingerManager.Inst.Singers.Values
                    .Where(singer => singer.SingerType == USingerType.Classic)
                    .OrderBy(singer => singer.Id, StringComparer.Ordinal)
                    .Select(singer => new { id = singer.Id, name = singer.Name });
                Console.WriteLine(JsonSerializer.Serialize(voices,
                    new JsonSerializerOptions { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping }));
                return 0;
            }
            Render(args[1], args[2], doc, callbacks);
            return 0;
        } catch (Exception error) {
            Console.Error.WriteLine(error.Message);
            return 1;
        }
    }

    private static void Render(
        string input, string output, DocManager doc, BlockingCollection<Action> callbacks
    ) {
        if (!string.Equals(Path.GetExtension(input), ".ustx", StringComparison.OrdinalIgnoreCase) ||
            !string.Equals(Path.GetExtension(output), ".wav", StringComparison.OrdinalIgnoreCase) ||
            File.Exists(output)) {
            throw new ArgumentException("Use a .ustx input and a new .wav output.");
        }
        var errors = new RenderErrors();
        doc.AddSubscriber(errors);
        UProject project = Ustx.Load(input);
        if (project.tracks.Count != 1 || project.parts.Count == 0 ||
            project.parts.Any(part => part is not UVoicePart)) {
            throw new ArgumentException("Render one singing track without audio parts.");
        }
        var track = project.tracks[0];
        if (track.Singer is not { Found: true, SingerType: USingerType.Classic } ||
            track.RendererSettings.renderer != "WORLDLINE-R") {
            throw new ArgumentException("Select an installed Classic singer and the WORLDLINE-R renderer.");
        }
        doc.ExecuteCmd(new LoadProjectNotification(project));
        project.ValidateFull();
        var voiceParts = project.parts.Cast<UVoicePart>().ToArray();
        var wait = Stopwatch.StartNew();
        while (voiceParts.Any(part => !part.PhonemesUpToDate)) {
            if (wait.Elapsed > TimeSpan.FromSeconds(60)) {
                throw new TimeoutException("OpenUtau did not finish phonemizing the project.");
            }
            if (callbacks.TryTake(out var callback, 100)) {
                callback();
            }
            errors.Check();
        }
        if (voiceParts.Any(part => part.notes.Count == 0 || part.notes.Any(note => note.Error))) {
            throw new InvalidDataException("The voicebank could not pronounce one or more notes.");
        }
        var temporary = output + "." + Guid.NewGuid().ToString("N") + ".tmp.wav";
        var stem = PathManager.Inst.GetExportPath(temporary, track);
        try {
            var task = PlaybackManager.Inst.RenderToFiles(project, temporary);
            while (!task.IsCompleted) {
                if (callbacks.TryTake(out var callback, 100)) {
                    callback();
                }
            }
            task.GetAwaiter().GetResult();
            while (callbacks.TryTake(out var callback)) {
                callback();
            }
            errors.Check();
            using (var wave = new WaveFileReader(stem)) {
                if (wave.Length == 0 || wave.WaveFormat.Channels != 1 ||
                    wave.WaveFormat.SampleRate != 44100 || wave.WaveFormat.BitsPerSample != 16) {
                    throw new InvalidDataException("OpenUtau did not produce a mono vocal stem.");
                }
            }
            File.Move(stem, output);
        } finally {
            File.Delete(stem);
        }
    }

    private sealed class RenderScheduler(BlockingCollection<Action> callbacks) : TaskScheduler {
        protected override void QueueTask(Task task) => callbacks.Add(() => {
            TryExecuteTask(task);
            task.GetAwaiter().GetResult();
        });

        protected override bool TryExecuteTaskInline(Task task, bool wasQueued) => false;
        protected override IEnumerable<Task> GetScheduledTasks() => [];
    }

    private sealed class RenderErrors : ICmdSubscriber {
        private Exception? error;

        public void OnNext(UCommand command, bool isUndo) {
            if (command is ErrorMessageNotification message) {
                error = message.e ?? new InvalidOperationException(message.message);
            }
        }

        public void Check() {
            if (error is not null) {
                throw new InvalidOperationException(error.Message, error);
            }
        }
    }
}
