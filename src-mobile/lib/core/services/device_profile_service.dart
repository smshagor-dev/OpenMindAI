import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';

import '../constants/model_catalog.dart';

class MobileDeviceProfile {
  const MobileDeviceProfile({
    required this.deviceName,
    required this.platform,
    required this.osVersion,
    required this.ramMb,
    required this.freeDiskBytes,
    required this.recommendedModel,
  });

  final String deviceName;
  final String platform;
  final String osVersion;
  final int ramMb;
  final int freeDiskBytes;
  final MobileModel recommendedModel;

  int get ramGb => (ramMb / 1024).round();
  double get freeDiskGb => freeDiskBytes / (1024 * 1024 * 1024);
}

class DeviceProfileService {
  final DeviceInfoPlugin _deviceInfo = DeviceInfoPlugin();

  Future<MobileDeviceProfile> read() async {
    if (Platform.isAndroid) {
      final info = await _deviceInfo.androidInfo;
      final ramMb = info.physicalRamSize;
      final freeDiskBytes = info.freeDiskSize;
      return MobileDeviceProfile(
        deviceName: '${info.manufacturer} ${info.model}'.trim(),
        platform: 'Android',
        osVersion: info.version.release,
        ramMb: ramMb,
        freeDiskBytes: freeDiskBytes,
        recommendedModel: MobileModelCatalog.initialInstallModel(
          ramGb: _ramGb(ramMb),
          freeDiskBytes: freeDiskBytes,
        ),
      );
    }

    if (Platform.isIOS) {
      final info = await _deviceInfo.iosInfo;
      final ramMb = info.physicalRamSize;
      final freeDiskBytes = info.freeDiskSize;
      return MobileDeviceProfile(
        deviceName: info.modelName,
        platform: 'iOS',
        osVersion: info.systemVersion,
        ramMb: ramMb,
        freeDiskBytes: freeDiskBytes,
        recommendedModel: MobileModelCatalog.initialInstallModel(
          ramGb: _ramGb(ramMb),
          freeDiskBytes: freeDiskBytes,
        ),
      );
    }

    throw UnsupportedError(
      'OpenMindAI mobile currently targets Android and iOS.',
    );
  }

  int _ramGb(int ramMb) => (ramMb / 1024).floor();
}
