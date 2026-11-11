# CodexBar provider port

The provider contract, credential discovery rules, direct HTTP strategies, and
quota normalization in this crate are adapted from:

- https://github.com/steipete/CodexBar
- Source commit: 7447bc77c2497323d9c41e40164ce705533493c8
- Copyright (c) 2026 Peter Steinberger
- License: MIT (the complete notice is in LICENSE-CodexBar)

Reference paths:

- Sources/CodexBarCore/Providers/ProviderFetchPlan.swift
- Sources/CodexBarCore/UsageFetcher.swift
- Sources/CodexBarCore/Providers/Codex/CodexOAuth/
- Sources/CodexBarCore/Providers/Codex/CodexProviderDescriptor.swift
- Sources/CodexBarCore/Providers/Codex/CodexRateWindowNormalizer.swift
- Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/
- Sources/CodexBarCore/Providers/Claude/ClaudeConfigPaths.swift
- Sources/CodexBarCore/Resources/Plugins/zai.js
- Sources/CodexBarCore/Providers/Zai/ZaiSettingsReader.swift

This port implements direct OAuth/API quota collection. Browser automation,
interactive CLI/PTY probes, account switching, and local cost-log scanning are
not part of its collection path. Shared Codex and Claude CLI credentials remain
read-only, matching the reference's native credential ownership policy.

Herdr-specific integration is licensed under the repository's Apache-2.0 license.
