import 'dart:async';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:dio/dio.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

import '../constants/model_catalog.dart';

class InstalledMobileModel {
  const InstalledMobileModel({
    required this.model,
    required this.modelPath,
    this.mmprojPath,
  });

  final MobileModel model;
  final String modelPath;
  final String? mmprojPath;
}

class ModelInstallProgress {
  const ModelInstallProgress({
    required this.modelId,
    required this.stage,
    required this.progress,
    required this.receivedBytes,
    required this.totalBytes,
  });

  final String modelId;
  final String stage;
  final double progress;
  final int receivedBytes;
  final int totalBytes;
}

class ModelStorageService {
  ModelStorageService({Dio? dio}) : _dio = dio ?? Dio();

  static const int _freeSpaceReserveBytes = 768 * 1024 * 1024;

  final Dio _dio;
  final Map<String, CancelToken> _cancelTokens = {};

  Future<Directory> _modelDirectory(MobileModel model) async {
    final support = await getApplicationSupportDirectory();
    final directory = Directory(p.join(support.path, 'models', model.id));
    if (!await directory.exists()) {
      await directory.create(recursive: true);
    }
    return directory;
  }

  Future<InstalledMobileModel?> installed(MobileModel model) async {
    final directory = await _modelDirectory(model);
    if (!await directory.exists()) {
      return null;
    }
    final files = await directory
        .list()
        .where((entity) => entity is File)
        .cast<File>()
        .toList();
    File? weights;
    File? projector;
    for (final file in files) {
      final name = p.basename(file.path).toLowerCase();
      if (!name.endsWith('.gguf') || name.endsWith('.part')) {
        continue;
      }
      if (name.startsWith('mmproj-')) {
        projector = file;
      } else {
        weights ??= file;
      }
    }
    if (weights == null) {
      return null;
    }
    if (model.supportsVision && projector == null) {
      return null;
    }
    return InstalledMobileModel(
      model: model,
      modelPath: weights.path,
      mmprojPath: projector?.path,
    );
  }

  Future<bool> isInstalled(MobileModel model) async {
    return await installed(model) != null;
  }

  Future<InstalledMobileModel> install(
    MobileModel model, {
    required void Function(ModelInstallProgress progress) onProgress,
  }) async {
    final existing = await installed(model);
    if (existing != null) {
      return existing;
    }

    final directory = await _modelDirectory(model);
    final token = CancelToken();
    _cancelTokens[model.id]?.cancel('Superseded by a new install request.');
    _cancelTokens[model.id] = token;

    try {
      final artifacts = await _resolveArtifacts(model);
      await _ensureEnoughFreeSpace(model, artifacts);
      final weights = artifacts.weights;
      final weightPath = p.join(directory.path, weights.filename);
      await _downloadAndVerify(
        modelId: model.id,
        artifact: weights,
        destinationPath: weightPath,
        stage: 'Downloading model',
        token: token,
        onProgress: onProgress,
      );

      String? mmprojPath;
      if (artifacts.mmproj != null) {
        mmprojPath = p.join(directory.path, artifacts.mmproj!.filename);
        await _downloadAndVerify(
          modelId: model.id,
          artifact: artifacts.mmproj!,
          destinationPath: mmprojPath,
          stage: 'Downloading vision projector',
          token: token,
          onProgress: onProgress,
        );
      }

      onProgress(
        ModelInstallProgress(
          modelId: model.id,
          stage: 'Ready',
          progress: 1,
          receivedBytes: weights.size,
          totalBytes: weights.size,
        ),
      );
      return InstalledMobileModel(
        model: model,
        modelPath: weightPath,
        mmprojPath: mmprojPath,
      );
    } finally {
      _cancelTokens.remove(model.id);
    }
  }

  void cancelInstall(String modelId) {
    _cancelTokens[modelId]?.cancel('Download cancelled.');
  }

  Future<void> delete(MobileModel model) async {
    cancelInstall(model.id);
    final directory = await _modelDirectory(model);
    if (await directory.exists()) {
      await directory.delete(recursive: true);
    }
  }

  Future<void> _ensureEnoughFreeSpace(
    MobileModel model,
    _ResolvedArtifacts artifacts,
  ) async {
    final freeBytes = await _freeDiskBytes();
    if (freeBytes <= 0) {
      return;
    }
    final artifactBytes = artifacts.weights.size + (artifacts.mmproj?.size ?? 0);
    final requiredBytes = artifactBytes + _freeSpaceReserveBytes;
    if (freeBytes >= requiredBytes) {
      return;
    }

    final requiredGb = requiredBytes / 1024 / 1024 / 1024;
    final freeGb = freeBytes / 1024 / 1024 / 1024;
    throw ModelInstallException(
      '${model.name} needs about ${requiredGb.toStringAsFixed(1)} GB free including working space, but this device has ${freeGb.toStringAsFixed(1)} GB available.',
    );
  }

  Future<int> _freeDiskBytes() async {
    final deviceInfo = DeviceInfoPlugin();
    if (Platform.isAndroid) {
      return (await deviceInfo.androidInfo).freeDiskSize;
    }
    if (Platform.isIOS) {
      return (await deviceInfo.iosInfo).freeDiskSize;
    }
    return 0;
  }

  Future<_ResolvedArtifacts> _resolveArtifacts(MobileModel model) async {
    final response = await _dio.get<Map<String, dynamic>>(
      'https://huggingface.co/api/models/${model.repository}',
      queryParameters: const {'blobs': 'true'},
      options: Options(headers: const {'Accept': 'application/json'}),
    );
    final siblings =
        (response.data?['siblings'] as List?)?.whereType<Map>().toList() ??
        const [];
    if (siblings.isEmpty) {
      throw ModelInstallException(
        'Could not resolve files for ${model.name}.',
      );
    }

    _RemoteArtifact? select(
      List<String> required, {
      bool projector = false,
    }) {
      for (final raw in siblings) {
        final filename = raw['rfilename']?.toString() ?? '';
        final lower = filename.toLowerCase();
        if (projector && !lower.startsWith('mmproj-')) {
          continue;
        }
        if (!projector && lower.startsWith('mmproj-')) {
          continue;
        }
        if (!required.every((part) => lower.contains(part.toLowerCase()))) {
          continue;
        }
        final lfs = raw['lfs'] is Map
            ? Map<String, dynamic>.from(raw['lfs'] as Map)
            : null;
        final size =
            (lfs?['size'] as num?)?.toInt() ??
            (raw['size'] as num?)?.toInt() ??
            0;
        final oid = lfs?['oid']?.toString();
        return _RemoteArtifact(
          filename: filename,
          url:
              'https://huggingface.co/${model.repository}/resolve/main/${Uri.encodeComponent(filename).replaceAll('%2F', '/')}',
          sha256: oid?.replaceFirst('sha256:', ''),
          size: size,
        );
      }
      return null;
    }

    final weights = select(model.filenameContains);
    if (weights == null) {
      throw ModelInstallException(
        'Compatible weights are not currently available for ${model.name}.',
      );
    }
    final projector = model.mmprojFilenameContains == null
        ? null
        : select(model.mmprojFilenameContains!, projector: true);
    if (model.supportsVision && projector == null) {
      throw ModelInstallException(
        'Vision support files are not currently available for ${model.name}.',
      );
    }
    return _ResolvedArtifacts(weights: weights, mmproj: projector);
  }

  Future<void> _downloadAndVerify({
    required String modelId,
    required _RemoteArtifact artifact,
    required String destinationPath,
    required String stage,
    required CancelToken token,
    required void Function(ModelInstallProgress progress) onProgress,
  }) async {
    final finalFile = File(destinationPath);
    if (await finalFile.exists()) {
      if (artifact.sha256 == null ||
          await _sha256(finalFile) == artifact.sha256) {
        return;
      }
      await finalFile.delete();
    }

    final partFile = File('$destinationPath.part');
    if (await partFile.exists()) {
      await partFile.delete();
    }
    await _dio.download(
      artifact.url,
      partFile.path,
      cancelToken: token,
      deleteOnError: true,
      options: Options(
        followRedirects: true,
        receiveTimeout: const Duration(hours: 4),
        headers: const {'Accept': 'application/octet-stream'},
      ),
      onReceiveProgress: (received, total) {
        final expected = total > 0 ? total : artifact.size;
        onProgress(
          ModelInstallProgress(
            modelId: modelId,
            stage: stage,
            progress: expected > 0
                ? (received / expected).clamp(0, 1).toDouble()
                : 0,
            receivedBytes: received,
            totalBytes: expected,
          ),
        );
      },
    );

    if (artifact.sha256 != null) {
      onProgress(
        ModelInstallProgress(
          modelId: modelId,
          stage: 'Verifying download',
          progress: 1,
          receivedBytes: await partFile.length(),
          totalBytes: artifact.size,
        ),
      );
      final actual = await _sha256(partFile);
      if (actual.toLowerCase() != artifact.sha256!.toLowerCase()) {
        await partFile.delete();
        throw const ModelInstallException(
          'Downloaded model verification failed. Please try again.',
        );
      }
    }
    await partFile.rename(finalFile.path);
  }

  Future<String> _sha256(File file) async {
    return (await sha256.bind(file.openRead()).first).toString();
  }
}

class _ResolvedArtifacts {
  const _ResolvedArtifacts({required this.weights, this.mmproj});

  final _RemoteArtifact weights;
  final _RemoteArtifact? mmproj;
}

class _RemoteArtifact {
  const _RemoteArtifact({
    required this.filename,
    required this.url,
    required this.sha256,
    required this.size,
  });

  final String filename;
  final String url;
  final String? sha256;
  final int size;
}

class ModelInstallException implements Exception {
  const ModelInstallException(this.message);

  final String message;

  @override
  String toString() => message;
}
