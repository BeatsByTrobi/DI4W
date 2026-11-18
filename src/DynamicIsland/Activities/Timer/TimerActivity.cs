using System.Diagnostics;
using System.IO;
using System.Media;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Threading;
using DynamicIsland.Core;
using DynamicIsland.Settings;
using DynamicIsland.UI;
using Activity = DynamicIsland.Core.Activity;

namespace DynamicIsland.Activities.Timer;

/// <summary>Countdown timer: orange icon + orange countdown in the pill, iOS-style controls when expanded.</summary>
public sealed class TimerActivity : Activity
{
    private readonly ActivityManager _manager;
    private readonly AppSettings _settings;
    private readonly DispatcherTimer _tick;
    private readonly Stopwatch _stopwatch = new();
    private TimeSpan _duration;
    private SoundPlayer? _alarm;

    private readonly TextBlock _compactText;
    private readonly TextBlock _expandedText;
    private readonly ContentControl _pauseIcon;
    private readonly TextBlock _minimalText;

    public TimerActivity(ActivityManager manager, AppSettings settings)
    {
        _manager = manager;
        _settings = settings;
        _tick = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(200) };
        _tick.Tick += (_, _) => Update();

        Leading = Ui.Glyph(Icons.TimerGlyph, 15, Ui.Orange);

        _compactText = Ui.Text("0:00", 15, FontWeights.SemiBold, Ui.Orange, tabular: true);
        Trailing = _compactText;

        // Minimal: the time left, as the iPhone's Timer shows it (an oval up to 45 wide).
        _minimalText = Ui.Text("0:00", 13, FontWeights.SemiBold, Ui.Orange, tabular: true);
        _minimalText.HorizontalAlignment = HorizontalAlignment.Center;
        Bubble = _minimalText;

        _expandedText = Ui.Text("0:00", 44, FontWeights.Light, Ui.Orange, tabular: true);
        _pauseIcon = new ContentControl();
        Expanded = BuildExpanded();
    }

    public override string Name => L.T("Timer");
    public override int Priority => 30;
    public override string Kind => "Timer";
    public override double CompactWidth => 230;
    public override double MinimalWidth => 45;
    public override Color? Keyline => Ui.Orange.Color;
    public override Size ExpandedSize => new(Ui.AlertWidth, 100);

    public override FrameworkElement Leading { get; }
    public override FrameworkElement Trailing { get; }
    public override FrameworkElement Bubble { get; }
    public override FrameworkElement Expanded { get; }

    public bool IsRunning => _stopwatch.IsRunning;
    public TimeSpan Remaining => _duration - _stopwatch.Elapsed;

    public void Start(TimeSpan duration)
    {
        StopAlarm();
        _duration = duration;
        _stopwatch.Restart();
        _tick.Start();
        IsActive = true;
        Update();
    }

    public void TogglePause()
    {
        if (_stopwatch.IsRunning) _stopwatch.Stop();
        else _stopwatch.Start();
        Update();
    }

    public void Cancel()
    {
        _stopwatch.Reset();
        _tick.Stop();
        IsActive = false;
    }

    private void Update()
    {
        var remaining = Remaining;
        if (remaining <= TimeSpan.Zero)
        {
            Finish();
            return;
        }

        // Round up so "5:00" shows for the whole first second, like iOS.
        var shown = TimeSpan.FromSeconds(Math.Ceiling(remaining.TotalSeconds));
        var text = Ui.FormatTime(shown);
        _compactText.Text = text;
        _expandedText.Text = text;
        _minimalText.Text = text;
        _minimalText.FontSize = text.Length > 5 ? 10.5 : 13; // "1:02:03" still fits the oval

        var paused = !_stopwatch.IsRunning;
        _compactText.Opacity = _minimalText.Opacity = paused ? 0.55 : 1;
        _expandedText.Opacity = paused ? 0.55 : 1;
        _pauseIcon.Content = Ui.Shape(paused ? Icons.Play : Icons.Pause, 15, Ui.Orange);

    }

    private void Finish()
    {
        _tick.Stop();
        _stopwatch.Reset();
        IsActive = false;

        var label = Ui.FormatTime(_duration);
        var icon = Ui.GlyphCircle(Icons.TimerGlyph, Ui.OrangeDim, Ui.IconSize, 22);
        ((TextBlock)icon.Child).Foreground = Ui.Orange;

        // Pulse the icon while ringing.
        icon.RenderTransformOrigin = new Point(0.5, 0.5);
        var pulse = new ScaleTransform();
        icon.RenderTransform = pulse;
        var anim = new DoubleAnimation(1, 1.12, TimeSpan.FromMilliseconds(450))
        {
            AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever,
            EasingFunction = new SineEase(),
        };
        pulse.BeginAnimation(ScaleTransform.ScaleXProperty, anim);
        pulse.BeginAnimation(ScaleTransform.ScaleYProperty, anim);

        IslandAlert? alert = null;
        var stop = Ui.CircleButton(Ui.Text(L.T("Stop"), 14, FontWeights.SemiBold, Brushes.Black), Ui.Orange, Ui.IconSize, () =>
        {
            if (alert is not null) _manager.DismissAlert(alert);
        });
        stop.Padding = new Thickness(0);

        alert = new IslandAlert
        {
            Content = Ui.AlertView(icon, L.T("Timer"), L.F("{0} is done", label), Ui.Orange, stop),
            Sticky = true,
            BreaksThroughQuiet = true,
            Keyline = Ui.Orange.Color,
            OnDismissed = StopAlarm,
        };
        _manager.ShowAlert(alert);
        PlayAlarm();
    }

    private void PlayAlarm()
    {
        if (!_settings.Sounds) return;
        try
        {
            var wav = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "Media", "Alarm01.wav");
            if (!File.Exists(wav)) wav = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "Media", "Windows Notify System Generic.wav");
            _alarm = new SoundPlayer(wav);
            _alarm.PlayLooping();
        }
        catch (Exception ex)
        {
            Log.Error("Could not play timer alarm", ex);
        }
    }

    private void StopAlarm()
    {
        _alarm?.Stop();
        _alarm?.Dispose();
        _alarm = null;
    }

    private FrameworkElement BuildExpanded()
    {
        // Layout follows the iOS timer live activity: round controls left, big orange countdown right.
        var grid = new Grid { Margin = new Thickness(18, 16, 22, 16) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var buttons = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        var pause = Ui.CircleButton(_pauseIcon, Ui.OrangeDim, 52, TogglePause, L.T("Pause / resume"));
        var cancel = Ui.CircleButton(Icons.Stroke(Icons.Close, 14, Ui.Primary, 2.4), Ui.Track, 52, Cancel, L.T("Cancel"));
        cancel.Margin = new Thickness(12, 0, 0, 0);
        buttons.Children.Add(pause);
        buttons.Children.Add(cancel);
        grid.Children.Add(buttons);

        var right = new StackPanel { HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        var label = Ui.Text(L.T("Timer"), 14, FontWeights.SemiBold, Ui.Orange);
        label.HorizontalAlignment = HorizontalAlignment.Right;
        _expandedText.HorizontalAlignment = HorizontalAlignment.Right;
        _expandedText.Margin = new Thickness(0, -6, 0, -4);
        right.Children.Add(label);
        right.Children.Add(_expandedText);
        Grid.SetColumn(right, 1);
        grid.Children.Add(right);

        return grid;
    }
}
