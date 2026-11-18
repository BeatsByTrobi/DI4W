using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Shapes;

namespace DynamicIsland.UI;

/// <summary>
/// Whether the island is light (set once at startup, before any view is built; switching restarts the island,
/// because views are built once and reused).
/// </summary>
public static class Palette
{
    public static bool Light { get; set; }
}

/// <summary>Small factory helpers so every view uses the same iOS-like typography, colors and controls.</summary>
public static class Ui
{
    /// <summary>Apple's San Francisco when it's installed; Windows' own Segoe UI Variable otherwise.</summary>
    public static readonly FontFamily Font = new("SF Pro Text, SF Pro Display, Segoe UI Variable Text, Segoe UI");
    public static readonly FontFamily IconFont = new("Segoe Fluent Icons, Segoe MDL2 Assets");

    /// <summary>The pill itself: black, or iOS' light grey-white.</summary>
    public static readonly SolidColorBrush PillBackground = Themed("#000000", "#F5F5F7");

    /// <summary>Main text and icons on the pill: white on the dark island, near-black on the light one.</summary>
    public static readonly SolidColorBrush Primary = Themed("#FFFFFF", "#1C1C1E");

    /// <summary>Icons on a colored circle or button (always white).</summary>
    public static readonly SolidColorBrush OnColor = Freeze(new SolidColorBrush(Colors.White));

    /// <summary>iOS' secondaryLabel: the second line of an alert, captions, times.</summary>
    public static readonly SolidColorBrush Grey = Themed("#99EBEBF5", "#993C3C43");
    public static readonly SolidColorBrush Orange = Brush("#FF9F0A");
    public static readonly SolidColorBrush OrangeDim = Themed("#4A3410", "#FFE2B8");
    public static readonly SolidColorBrush AmberDim = Themed("#4A4010", "#FFF1B0");
    public static readonly SolidColorBrush Green = Brush("#30D158");
    public static readonly SolidColorBrush Amber = Brush("#FFD60A");
    public static readonly SolidColorBrush Track = Themed("#3A3A3C", "#D1D1D6");
    public static readonly SolidColorBrush ButtonBg = Themed("#2C2C2E", "#E5E5EA");
    public static readonly SolidColorBrush DiscordBlurple = Brush("#5865F2");
    public static readonly SolidColorBrush Blue = Brush("#0A84FF");
    public static readonly SolidColorBrush Red = Brush("#FF453A");
    public static readonly SolidColorBrush Purple = Brush("#BF5AF2");
    public static readonly SolidColorBrush Teal = Brush("#64D2FF");
    public static readonly SolidColorBrush Indigo = Brush("#5E5CE6");

    // Apple's expanded presentation, in points: 371 wide, 84 tall at the least. Content sits at an even 16 from the
    // edges, which with a 52 icon makes the icon concentric with the island's rounded end.
    public const double AlertWidth = 371;
    public const double AlertHeight = 84;
    public const double IconSize = 52;
    private const double Margin = 16;

    /// <summary>Height of an alert with a row of buttons under it (<see cref="ActionAlertView"/>).</summary>
    public const double ActionAlertHeight = Margin + IconSize + 12 + ActionHeight + Margin;
    private const double ActionHeight = 44;

    public static SolidColorBrush Brush(string hex) =>
        Freeze(new SolidColorBrush((Color)ColorConverter.ConvertFromString(hex)));

    private static SolidColorBrush Themed(string dark, string light) => Brush(Palette.Light ? light : dark);

    private static T Freeze<T>(T f) where T : Freezable
    {
        f.Freeze();
        return f;
    }

    public static TextBlock Text(string text, double size, FontWeight weight, Brush brush, bool tabular = false)
    {
        var tb = new TextBlock
        {
            Text = text,
            FontFamily = Font,
            FontSize = size,
            FontWeight = weight,
            Foreground = brush,
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (tabular) Typography.SetNumeralAlignment(tb, FontNumeralAlignment.Tabular);
        TextOptions.SetTextFormattingMode(tb, TextFormattingMode.Ideal);
        return tb;
    }

    public static TextBlock Glyph(string glyph, double size, Brush brush) => new()
    {
        Text = glyph,
        FontFamily = IconFont,
        FontSize = size,
        Foreground = brush,
        VerticalAlignment = VerticalAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Center,
    };

    public static Path Shape(Geometry geometry, double size, Brush brush) => new()
    {
        Data = geometry,
        Fill = brush,
        Stroke = brush,
        StrokeThickness = 1.6,
        StrokeLineJoin = PenLineJoin.Round,
        Stretch = Stretch.Uniform,
        Width = size,
        Height = size,
        VerticalAlignment = VerticalAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Center,
    };

    /// <summary>A button with no chrome: the content is the whole button, with hover/press feedback.</summary>
    public static Button Button(object content, Action onClick, Brush? background = null, double width = double.NaN,
                                double height = double.NaN, string? tooltip = null)
    {
        var b = new Button
        {
            Content = content,
            Background = background ?? Brushes.Transparent,
            Width = width,
            Height = height,
            ToolTip = tooltip,
            Style = (Style)Application.Current.FindResource("IslandButton"),
        };
        b.Click += (_, _) => onClick();
        // Screen readers: icon-only buttons need a name; use the tooltip, or the label text.
        var name = tooltip ?? (content as TextBlock)?.Text;
        if (name is not null) System.Windows.Automation.AutomationProperties.SetName(b, name);
        b.SizeChanged += (_, _) =>
        {
            if (b.Template?.FindName("Bg", b) is Border bg)
            {
                bg.CornerRadius = new CornerRadius(Math.Min(b.ActualWidth, b.ActualHeight) / 2);
            }
        };
        return b;
    }

    /// <summary>Round button as used in iOS live activities (timer pause / cancel).</summary>
    public static Button CircleButton(UIElement icon, Brush background, double size, Action onClick, string? tooltip = null) =>
        Button(icon, onClick, background, size, size, tooltip);

    /// <summary>Rounded image (album art / app icons).</summary>
    public static Border Image(ImageSource? source, double size, double radius)
    {
        var border = new Border
        {
            Width = size,
            Height = size,
            CornerRadius = new CornerRadius(radius),
            Background = source is null ? ButtonBg : new ImageBrush(source) { Stretch = Stretch.UniformToFill },
            VerticalAlignment = VerticalAlignment.Center,
        };
        RenderOptions.SetBitmapScalingMode(border, BitmapScalingMode.HighQuality);
        return border;
    }

    /// <summary>
    /// The standard expanded alert, laid out like Apple's: a 52 icon at an even 16 from the edges, a heavy title, a
    /// secondary-label second line, and an optional accessory (round buttons) on the trailing side.
    /// </summary>
    public static FrameworkElement AlertView(UIElement icon, string title, string subtitle,
                                             Brush? titleBrush = null, UIElement? accessory = null)
    {
        var grid = new Grid { Margin = new Thickness(Margin), Height = IconSize, VerticalAlignment = VerticalAlignment.Top };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var iconHost = new Border { Width = IconSize, Height = IconSize, VerticalAlignment = VerticalAlignment.Center, Child = icon };
        grid.Children.Add(iconHost);

        var stack = new StackPanel { Margin = new Thickness(12, 0, accessory is null ? 4 : 10, 0), VerticalAlignment = VerticalAlignment.Center };
        stack.Children.Add(Text(title, 16, FontWeights.SemiBold, titleBrush ?? Primary));
        if (!string.IsNullOrWhiteSpace(subtitle))
        {
            var sub = Text(subtitle, 14.5, FontWeights.Normal, Grey);
            sub.TextWrapping = TextWrapping.Wrap;
            sub.MaxHeight = 38; // two lines
            sub.Margin = new Thickness(0, 1, 0, 0);
            stack.Children.Add(sub);
        }
        Grid.SetColumn(stack, 1);
        grid.Children.Add(stack);

        if (accessory is not null)
        {
            var host = new ContentControl { Content = accessory, VerticalAlignment = VerticalAlignment.Center };
            Grid.SetColumn(host, 2);
            grid.Children.Add(host);
        }
        return grid;
    }

    /// <summary>An alert with a row of wide capsule buttons under it, like the iPhone's expanded prompts.
    /// Use with <see cref="ActionAlertHeight"/>.</summary>
    public static FrameworkElement ActionAlertView(UIElement icon, string title, string subtitle, params Button[] actions)
    {
        var header = AlertView(icon, title, subtitle);
        header.Margin = new Thickness(Margin, Margin, Margin, 0);
        var row = new System.Windows.Controls.Primitives.UniformGrid
        {
            Rows = 1,
            Margin = new Thickness(Margin - 4, 12, Margin - 4, Margin),
            Height = ActionHeight,
        };
        foreach (var action in actions)
        {
            action.Margin = new Thickness(4, 0, 4, 0);
            action.Height = ActionHeight;
            row.Children.Add(action);
        }
        var stack = new StackPanel();
        stack.Children.Add(header);
        stack.Children.Add(row);
        return stack;
    }

    /// <summary>A wide capsule button for <see cref="ActionAlertView"/>: grey, or filled with a color for the main action.</summary>
    public static Button ActionButton(string label, Action onClick, Brush? fill = null)
    {
        var text = Text(label, 16, FontWeights.SemiBold, fill is null ? Primary : Brushes.White);
        var button = Button(text, onClick, fill ?? ButtonBg);
        System.Windows.Automation.AutomationProperties.SetName(button, label);
        return button;
    }

    /// <summary>A filled circle with a glyph in the middle (fallback app icon). White glyph on a color; on the
    /// neutral grey the glyph follows the theme.</summary>
    public static Border GlyphCircle(string glyph, Brush background, double size, double glyphSize) => new()
    {
        Width = size,
        Height = size,
        CornerRadius = new CornerRadius(size / 2),
        Background = background,
        Child = Glyph(glyph, glyphSize, background == ButtonBg || background == Track ? Primary : OnColor),
    };

    public static string FormatTime(TimeSpan t)
    {
        if (t < TimeSpan.Zero) t = TimeSpan.Zero;
        return t.TotalHours >= 1
            ? $"{(int)t.TotalHours}:{t.Minutes:00}:{t.Seconds:00}"
            : $"{(int)t.TotalMinutes}:{t.Seconds:00}";
    }
}

/// <summary>Vector icons drawn to resemble SF Symbols.</summary>
public static class Icons
{
    public static readonly Geometry Play = Parse("M 1,0 L 13,7.5 L 1,15 Z");
    public static readonly Geometry Pause = Parse("M 0,0 H 4.5 V 15 H 0 Z M 8.5,0 H 13 V 15 H 8.5 Z");
    public static readonly Geometry Next = Parse("M 0,0 L 9,6.5 L 0,13 Z M 9,0 L 18,6.5 L 9,13 Z");
    public static readonly Geometry Previous = Parse("M 18,0 L 9,6.5 L 18,13 Z M 9,0 L 0,6.5 L 9,13 Z");
    public static readonly Geometry Close = Parse("M 0,0 L 12,12 M 12,0 L 0,12");
    public static readonly Geometry Check = Parse("M 0,7 L 5,12 L 14,1");

    public const string TimerGlyph = "";   // Stopwatch
    public const string ChatGlyph = "";    // Message
    public const string UpdateGlyph = ""; // Download
    public const string BellGlyph ="";    // Ringer

    private static Geometry Parse(string data)
    {
        var g = Geometry.Parse(data);
        g.Freeze();
        return g;
    }

    /// <summary>Stroke-only icon (close / check).</summary>
    public static Path Stroke(Geometry g, double size, Brush brush, double thickness = 2.2) => new()
    {
        Data = g,
        Stroke = brush,
        StrokeThickness = thickness,
        StrokeStartLineCap = PenLineCap.Round,
        StrokeEndLineCap = PenLineCap.Round,
        StrokeLineJoin = PenLineJoin.Round,
        Stretch = Stretch.Uniform,
        Width = size,
        Height = size,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
    };

    /// <summary>iOS-style battery: outline, fill for the charge level, a nub, and a bolt while charging.</summary>
    public static FrameworkElement Battery(double fraction, bool charging, Brush fill, double width = 27)
    {
        var height = width * 0.48;
        var body = new Border
        {
            Width = width, Height = height,
            CornerRadius = new CornerRadius(height * 0.3),
            BorderBrush = Ui.Grey, BorderThickness = new Thickness(1.2),
            Padding = new Thickness(1.6),
            Child = new Border
            {
                Width = Math.Max(2, (width - 6.4) * Math.Clamp(fraction, 0, 1)),
                HorizontalAlignment = HorizontalAlignment.Left,
                CornerRadius = new CornerRadius(height * 0.18),
                Background = fill,
            },
        };
        var nub = new Border
        {
            Width = 2, Height = height * 0.36, Margin = new Thickness(1, 0, 0, 0),
            CornerRadius = new CornerRadius(0, 1, 1, 0), Background = Ui.Grey, VerticalAlignment = VerticalAlignment.Center,
        };
        var grid = new Grid { VerticalAlignment = VerticalAlignment.Center };
        grid.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { body, nub } });
        if (charging)
        {
            grid.Children.Add(new Path
            {
                Data = Bolt, Fill = Ui.OnColor, Stroke = Brushes.Black, StrokeThickness = 0.8,
                Stretch = Stretch.Uniform, Height = height * 0.95, Margin = new Thickness(0, 0, 3, 0),
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            });
        }
        return grid;
    }

    private static readonly Geometry Bolt = Parse("M 6,0 L 0,8 H 4 L 2,14 L 9,5 H 5 Z");

    /// <summary>Circular progress arc starting at 12 o'clock, clockwise; <paramref name="progress"/> is the filled part 0..1.</summary>
    public static Path Ring(double progress, double size, double stroke, Brush brush)
    {
        var r = (size - stroke) / 2;
        var p = Math.Clamp(progress, 0.0001, 0.9999);
        var angle = p * 2 * Math.PI;
        var start = new Point(size / 2, stroke / 2);
        var end = new Point(size / 2 + r * Math.Sin(angle), size / 2 - r * Math.Cos(angle));
        var figure = new PathFigure { StartPoint = start, IsClosed = false };
        figure.Segments.Add(new ArcSegment(end, new Size(r, r), 0, angle > Math.PI, SweepDirection.Clockwise, true));
        return new Path
        {
            Data = new PathGeometry(new[] { figure }),
            Stroke = brush,
            StrokeThickness = stroke,
            StrokeStartLineCap = PenLineCap.Round,
            StrokeEndLineCap = PenLineCap.Round,
            Width = size,
            Height = size,
        };
    }
}
