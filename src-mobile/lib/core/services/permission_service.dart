import 'dart:io';
import 'package:permission_handler/permission_handler.dart';

class PermissionRequestResult {
  const PermissionRequestResult(this.statuses);
  final Map<Permission, PermissionStatus> statuses;

  bool get hasPermanentDenial =>
      statuses.values.any((status) => status.isPermanentlyDenied);
}

class PermissionService {
  /// Requests only capabilities the mobile app actually uses. Files are opened
  /// through system pickers, so broad storage access is deliberately avoided.
  Future<PermissionRequestResult> requestInitialPermissions() async {
    final permissions = <Permission>[
      Permission.camera,
      Permission.microphone,
      Permission.notification,
      if (Platform.isIOS) Permission.photos,
    ];

    final statuses = await permissions.request();
    return PermissionRequestResult(statuses);
  }

  Future<void> openSettings() async {
    await openAppSettings();
  }
}
