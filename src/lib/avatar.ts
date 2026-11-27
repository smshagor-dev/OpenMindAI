const MAX_AVATAR_DIMENSION = 1024;

export function readImageAsDataUrl(file: File, maxDimension = 256): Promise<string> {
  return new Promise((resolve, reject) => {
    if (!Number.isFinite(maxDimension) || maxDimension <= 0) {
      reject(new Error("Invalid avatar output dimension"));
      return;
    }
    const targetDimension = Math.min(MAX_AVATAR_DIMENSION, Math.floor(maxDimension));
    if (targetDimension < 1) {
      reject(new Error("Invalid avatar output dimension"));
      return;
    }

    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Failed to read image file"));
    reader.onload = () => {
      const image = new Image();
      image.onerror = () => reject(new Error("Failed to decode image file"));
      image.onload = () => {
        try {
          if (
            !Number.isFinite(image.width) || !Number.isFinite(image.height) ||
            image.width <= 0 || image.height <= 0
          ) {
            reject(new Error("Invalid decoded avatar dimensions"));
            return;
          }
          const scale = Math.min(1, targetDimension / Math.max(image.width, image.height));
          const width = Math.max(1, Math.round(image.width * scale));
          const height = Math.max(1, Math.round(image.height * scale));
          const canvas = document.createElement("canvas");
          canvas.width = width;
          canvas.height = height;
          const context = canvas.getContext("2d");
          if (!context) {
            reject(new Error("Canvas not supported"));
            return;
          }
          context.drawImage(image, 0, 0, width, height);
          resolve(canvas.toDataURL("image/jpeg", 0.85));
        } catch {
          reject(new Error("Failed to render avatar image"));
        }
      };
      image.src = reader.result as string;
    };
    try {
      reader.readAsDataURL(file);
    } catch {
      reject(new Error("Failed to read image file"));
    }
  });
}
