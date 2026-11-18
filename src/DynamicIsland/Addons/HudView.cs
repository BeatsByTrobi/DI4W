using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using DynamicIsland.Core;
using DynamicIsland.UI;

namespace DynamicIsland.Addons;

/// <summary>
/// A system indicator the way the iPhone shows one (Silent mode, Focus, charging): the island just widens, staying
/// one line tall, with an icon on the leading side and a level bar or a short text on the trailing side. One view
/// and one alert are reused, so holding a key updates the bar in place instead of queuing pop-ups.
/// </summary>
public sealed class HudView
{
    private const double BarWidth = 170;

    private readonly TextBlock _icon;
    private readonly Border _track;
    private readonly Border _fill;
    private readonly TextBlock _value;

    public HudView(string glyph, Brush accent)
    {
        _icon = Ui.Glyph(glyph, 17, accent);
        _fill = new Border { Height = 6, CornerRadius = new CornerRadius(3), Background = Ui.Primary, HorizontalAlignment = HorizontalAlignment.Left };
        _track = new Border { Width = BarWidth, Height = 6, CornerRadius = new CornerRadius(3), Background = Ui.Track, Child = _fill };
        _value = Ui.Text("", 15, FontWeights.SemiBold, accent == Ui.Primary ? Ui.Primary : accent);
        _value.TextAlignment = TextAlignment.Right;

        var trailing = new Grid { Children = { _track, _value } };
        Alert = new IslandAlert
        {
            Content = Ui.Indicator(_icon, trailing),
            Size = new Size(Ui.IndicatorWidth, Ui.IndicatorHeight),
            Duration = TimeSpan.FromSeconds(1.6),
            Urgent = true,
            Keyline = accent == Ui.Primary ? null : (accent as SolidColorBrush)?.Color,
        };
    }

    public IslandAlert Alert { get; }

    /// <summary>Level 0..1 as a bar (volume, brightness), or no level and a short text ("Caps Lock on").</summary>
    public void Set(double? level, string value, string? glyph = null)
    {
        if (glyph is not null) _icon.Text = glyph;
        _track.Visibility = level is null ? Visibility.Collapsed : Visibility.Visible;
        _value.Visibility = level is null ? Visibility.Visible : Visibility.Collapsed;
        _fill.Width = BarWidth * Math.Clamp(level ?? 0, 0, 1);
        _value.Text = value;
        // The bar has no words of its own; screen readers and the history read the value from the tooltip.
        _track.ToolTip = level is null ? null : value;
    }
}
