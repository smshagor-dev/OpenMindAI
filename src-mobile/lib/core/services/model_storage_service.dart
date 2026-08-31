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
  static const int _maxDownloadAttempts = 4;

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
    if (await partFile.exists() && artifact.size > 0) {
      final partialLength = await partFile.length();
      if (partialLength > artifact.size) {
        await partFile.delete();
      } else if (partialLength == artifact.size) {
        await _verifyAndCommit(
          modelId: modelId,
          artifact: artifact,
          partFile: partFile,
          finalFile: finalFile,
          onProgress: onProgress,
        );
        return;
      }
    }

    Object? lastError;
    for (var attempt = 1; attempt <= _maxDownloadAttempts; attempt++) {
      if (token.isCancelled) {
        throw DioException.requestCancelled(
          requestOptions: RequestOptions(path: artifact.url),
          reason: 'Download cancelled.',
        );
      }
      try {
        await _downloadRange(
          modelId: modelId,
          artifact: artifact,
          partFile: partFile,
          stage: stage,
          token: token,
          onProgress: onProgress,
        );
        await _verifyAndCommit(
          modelId: modelId,
          artifact: artifact,
          partFile: partFile,
          finalFile: finalFile,
          onProgress: onProgress,
        );
        return;
      } on DioException catch (error) {
        if (CancelToken.isCancel(error) || token.isCancelled) rethrow;
        lastError = error;
      } on SocketException catch (error) {
        lastError = error;
      }

      if (attempt < _maxDownloadAttempts) {
        onProgress(
          ModelInstallProgress(
            modelId: modelId,
            stage: 'Connection interrupted. Resuming…',
            progress: artifact.size > 0 && await partFile.exists()
                ? ((await partFile.length()) / artifact.size)
                    .clamp(0, 1)
                    .toDouble()
                : 0,
            receivedBytes:
                await partFile.exists() ? await partFile.length() : 0,
            totalBytes: artifact.size,
          ),
        );
        await Future<void>.delayed(Duration(seconds: attempt * 2));
      }
    }

    throw ModelInstallException(
      'Model download was interrupted repeatedly. Your progress is saved; try again to resume.${lastError == null ? '' : ' (${lastError.runtimeType})'}',
    );
  }

  Future<void> _downloadRange({
    required String modelId,
    required _RemoteArtifact artifact,
    required File partFile,
    required String stage,
    required CancelToken token,
    required void Function(ModelInstallProgress progress) onProgress,
  }) async {
    var existingBytes = await partFile.exists() ? await partFile.length() : 0;
    final headers = <String, String>{'Accept': 'application/octet-stream'};
    if (existingBytes > 0) {
      headers['Range'] = 'bytes=$existingBytes-';
    }

    final response = await _dio.get<ResponseBody>(
      artifact.url,
      cancelToken: token,
      options: Options(
        responseType: ResponseType.stream,
        followRedirects: true,
        receiveTimeout: const Duration(hours: 4),
        headers: headers,
        validateStatus: (status) =>
            status == 200 || status == 206 || status == 416,
      ),
    );

    if (response.statusCode == 416) {
      if (artifact.size > 0 && existingBytes == artifact.size) return;
      if (await partFile.exists()) await partFile.delete();
      existingBytes = 0;
      throw DioException.badResponse(
        statusCode: 416,
        requestOptions: response.requestOptions,
        response: response,
      );
    }

    final resumed = existingBytes > 0 && response.statusCode == 206;
    if (existingBytes > 0 && !resumed) {
      await partFile.writeAsBytes(const [], flush: true);
      existingBytes = 0;
    }

    final body = response.data;
    if (body == null) {
      throw DioException(
        requestOptions: response.requestOptions,
        response: response,
        message: 'Download server returned an empty response.',
      );
    }

    final sink = partFile.openWrite(
      mode: existingBytes > 0 ? FileMode.append : FileMode.write,
    );
    var received = existingBytes;
    try {
      await for (final chunk in body.stream) {
        if (token.isCancelled) {
          throw DioException.requestCancelled(
            requestOptions: response.requestOptions,
            reason: 'Download cancelled.',
          );
        }
        sink.add(chunk);
        received += chunk.length;
        final expected = artifact.size > 0
            ? artifact.size
            : existingBytes + body.contentLength;
        onProgress(
          ModelInstallProgress(
            modelId: modelId,
            stage: existingBytes > 0 ? 'Resuming model download' : stage,
            progress: expected > 0
                ? (received / expected).clamp(0, 1).toDouble()
                : 0,
            receivedBytes: received,
            totalBytes: expected,
          ),
        );
      }
    } finally {
      await sink.flush();
      await sink.close();
    }
  }

  Future<void> _verifyAndCommit({
    required String modelId,
    required _RemoteArtifact artifact,
    required File partFile,
    required File finalFile,
    required void Function(ModelInstallProgress progress) onProgress,
  }) async {
    if (!await partFile.exists()) {
      throw const ModelInstallException('Downloaded model data is missing.');
    }
    final size = await partFile.length();
    if (artifact.size > 0 && size != artifact.size) {
      throw ModelInstallException(
        'Model download is incomplete ($size of ${artifact.size} bytes). Progress was saved and can be resumed.',
      );
    }

    if (artifact.sha256 != null) {
      onProgress(
        ModelInstallProgress(
          modelId: modelId,
          stage: 'Verifying download',
          progress: 1,
          receivedBytes: size,
          totalBytes: artifact.size > 0 ? artifact.size : size,
        ),
      );
      final actual = await _sha256(partFile);
      if (actual.toLowerCase() != artifact.sha256!.toLowerCase()) {
        await partFile.delete();
        throw const ModelInstallException(
          'Downloaded model verification failed. The damaged partial file was removed; please try again.',
        );
      }
    }
    if (await finalFile.exists()) await finalFile.delete();
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
