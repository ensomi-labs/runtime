// What the Swift app's own types computed, derived from the core's data.

extension JudgementEvent {
    /// What the player is shown for the event's tier.
    public var judgement: Judgement { tierJudgement(tier: tier) }
}

extension TierCounts {
    public var judgedObjectCount: UInt32 { bigP + p1 + p2 + p3 + g + m }
    public var perfectCount: UInt32 { bigP }
    public var goodCount: UInt32 { p1 + p2 + p3 + g }
    public var missCount: UInt32 { m }
}
