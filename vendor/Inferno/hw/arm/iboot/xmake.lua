if has_config("apple_soc") then
	target("system_arm")
		add_files("boot.c", "dfu.c", "iboot.c", "inferno_pvgpu_boot.c", "kernel_patches.c", "patcher.c", "virt_patches.c")
	target_end()
end
