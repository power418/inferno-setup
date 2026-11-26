# iOS 14 Guest Patching (`inferno-fs-patcher --pvgpu`)

Untuk mengaktifkan jalur Metal terakselerasi di iOS 14 (`18A5351d`) tanpa
memaksa CoreImage/QuartzCore masuk ke mode software rendering:

1. Jalankan `inferno-fs-patcher` dengan flag `--pvgpu`:
   ```sh
   xmake r inferno-fs-patcher --pvgpu -d InfernoData/root
   ```
   Flag `--pvgpu` tetap menerapkan patch yang wajib untuk boot tanpa baseband/BT
   (`LaunchdPatches`, `BTAudioHALPluginPatches`, `QuartzCore` async dispatcher safety),
   namun **melewati** patch yang mematikan Metal (`CoreImagePatches`, nullifikasi
   `AccelServer::renderer`, `CMCapture`, `NeutrinoCore` SW renderer, `ClockKitUI`,
   dan `PosterBoard`).

2. Pasang `libInfernoMetal.dylib` ke `/usr/lib/libInfernoMetal.dylib` di root image
   dan arahkan interposition `MTLCreateSystemDefaultDevice` / `MTLCopyAllDevices`
   (atau `DYLD_INSERT_LIBRARIES=/usr/lib/libInfernoMetal.dylib` untuk app uji).
