import 'package:flutter/material.dart';

import 'app_theme.dart';

class OpenMindBrandMark extends StatelessWidget {
  const OpenMindBrandMark({super.key, this.size = 52, this.compact = false});

  final double size;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        gradient: const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [AppTheme.accentSoft, AppTheme.accent],
        ),
        borderRadius: BorderRadius.circular(compact ? size * .3 : size * .34),
        boxShadow: [
          BoxShadow(
            color: AppTheme.accent.withValues(alpha: .22),
            blurRadius: 24,
            spreadRadius: -7,
            offset: const Offset(0, 8),
          ),
        ],
      ),
      child: Icon(
        Icons.psychology_alt_rounded,
        size: size * .56,
        color: Colors.white,
      ),
    );
  }
}

class OpenMindSectionCard extends StatelessWidget {
  const OpenMindSectionCard({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(16),
    this.onTap,
  });

  final Widget child;
  final EdgeInsetsGeometry padding;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final card = Card(
      child: Padding(padding: padding, child: child),
    );
    if (onTap == null) return card;
    return InkWell(
      borderRadius: BorderRadius.circular(20),
      onTap: onTap,
      child: card,
    );
  }
}

class OpenMindPageHeader extends StatelessWidget {
  const OpenMindPageHeader({
    super.key,
    required this.title,
    this.subtitle,
    this.trailing,
  });

  final String title;
  final String? subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: Theme.of(context).textTheme.headlineMedium),
              if (subtitle != null) ...[
                const SizedBox(height: 6),
                Text(
                  subtitle!,
                  style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ],
          ),
        ),
        if (trailing != null) ...[const SizedBox(width: 12), trailing!],
      ],
    );
  }
}

class OpenMindFeatureIcon extends StatelessWidget {
  const OpenMindFeatureIcon(this.icon, {super.key, this.size = 42});

  final IconData icon;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: AppTheme.accent.withValues(alpha: .12),
        borderRadius: BorderRadius.circular(size * .32),
      ),
      child: Icon(icon, color: AppTheme.accent, size: size * .52),
    );
  }
}

class OpenMindStatusPill extends StatelessWidget {
  const OpenMindStatusPill({
    super.key,
    required this.label,
    this.icon,
    this.active = false,
  });

  final String label;
  final IconData? icon;
  final bool active;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
      decoration: BoxDecoration(
        color: active
            ? AppTheme.accent.withValues(alpha: .14)
            : scheme.surfaceContainer,
        borderRadius: BorderRadius.circular(999),
        border: Border.all(
          color: active
              ? AppTheme.accent.withValues(alpha: .45)
              : scheme.outlineVariant,
        ),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            Icon(icon, size: 15, color: active ? AppTheme.accent : null),
            const SizedBox(width: 6),
          ],
          Text(
            label,
            style: Theme.of(context).textTheme.labelMedium?.copyWith(
              fontWeight: FontWeight.w700,
              color: active ? AppTheme.accent : null,
            ),
          ),
        ],
      ),
    );
  }
}

class OpenMindEmptyState extends StatelessWidget {
  const OpenMindEmptyState({
    super.key,
    required this.icon,
    required this.title,
    required this.description,
    this.action,
  });

  final IconData icon;
  final String title;
  final String description;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 420),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 32),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              OpenMindFeatureIcon(icon, size: 58),
              const SizedBox(height: 18),
              Text(
                title,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.titleLarge,
              ),
              const SizedBox(height: 8),
              Text(
                description,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
              if (action != null) ...[const SizedBox(height: 20), action!],
            ],
          ),
        ),
      ),
    );
  }
}
