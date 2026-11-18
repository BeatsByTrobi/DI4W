using System.Windows;
using System.Windows.Media;

namespace DynamicIsland.Core;

/// <summary>
/// A live activity shown in the island, the Windows equivalent of an iOS Live Activity.
/// It supplies the compact leading/trailing views, the small "minimal" bubble view used
/// when two activities run at once, and the expanded view.
/// Each view is created once and reused (a WPF element can only have one parent).
/// </summary>
public abstract class Activity
{
    private bool _isActive;

    public event Action? Changed;

    public abstract string Name { get; }

    /// <summary>Extra words for screen readers when the compact view has no text of its own (e.g. the song).</summary>
    public virtual string? AccessibleDetail => null;

    /// <summary>Higher wins the main pill; the runner-up gets the bubble (unless the user set an order).</summary>
    public abstract int Priority { get; }

    /// <summary>Stable id for the user's order in the settings ("Timer", "Media"…), not translated.</summary>
    public virtual string Kind => Name;

    /// <summary>The mouse moved onto or off the compact pill while this activity is in it.</summary>
    public virtual void OnHover(bool hovering) { }

    public bool IsActive
    {
        get => _isActive;
        protected set
        {
            if (_isActive == value) return;
            _isActive = value;
            RaiseChanged();
        }
    }

    public abstract FrameworkElement Leading { get; }
    public abstract FrameworkElement Trailing { get; }
    public abstract FrameworkElement Bubble { get; }
    public abstract FrameworkElement Expanded { get; }

    /// <summary>Tint of the hairline around the island while this activity shows (Apple's key line); null is neutral.</summary>
    public virtual Color? Keyline => null;

    public virtual double CompactWidth => 230;

    /// <summary>Width of the minimal view's spot: a 37 circle, up to a 45 wide oval for text such as a countdown.</summary>
    public virtual double MinimalWidth => 37;
    public virtual Size ExpandedSize => new(371, 160);

    protected void RaiseChanged() => Changed?.Invoke();
}

/// <summary>A transient pop-out (notification, download complete, timer finished).</summary>
public sealed class IslandAlert
{
    public required FrameworkElement Content { get; init; }
    public Size Size { get; init; } = new(371, 84);

    /// <summary>Tint of the hairline around the island while this alert shows; null is neutral.</summary>
    public Color? Keyline { get; init; }
    public TimeSpan Duration { get; init; } = TimeSpan.FromSeconds(4.5);

    /// <summary>Never auto-dismisses; must be clicked / dismissed in code.</summary>
    public bool Sticky { get; set; }

    /// <summary>Stays until the mouse has hovered it once, then times out normally.</summary>
    public bool StickyUntilHover { get; set; }

    /// <summary>Shown right away, putting any current alert back in line (volume/brightness bars).</summary>
    public bool Urgent { get; init; }

    /// <summary>Still shown while Do Not Disturb is on (alarms, timers, calls).</summary>
    public bool BreaksThroughQuiet { get; init; }

    public Action? OnClick { get; init; }
    public Action? OnDismissed { get; init; }
}
