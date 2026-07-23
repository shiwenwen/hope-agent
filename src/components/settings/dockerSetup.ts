export type DockerHostOs = "macos" | "windows" | "linux" | "unknown" | string

export interface DockerStatus {
  installed: boolean
  running: boolean
  hostOs?: DockerHostOs
  backend?: "native" | "wsl" | null
  wslInstalled?: boolean | null
  wslDistributionInstalled?: boolean | null
  wslDockerInstalled?: boolean | null
}

interface DockerOption {
  label: string
  url: string
}

export function dockerInstallOptions(
  hostOs?: DockerHostOs,
  wslDistributionInstalled = false,
): {
  primary: DockerOption
  alternatives: DockerOption[]
} {
  switch (hostOs) {
    case "linux":
      return {
        primary: { label: "Docker Engine", url: "https://docs.docker.com/engine/install/" },
        alternatives: [
          {
            label: "Docker Desktop for Linux",
            url: "https://docs.docker.com/desktop/setup/install/linux/",
          },
          { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
        ],
      }
    case "windows":
      if (wslDistributionInstalled) {
        return {
          primary: {
            label: "Docker Engine on WSL",
            url: "https://docs.docker.com/engine/install/",
          },
          alternatives: [
            {
              label: "Docker Desktop + WSL2",
              url: "https://docs.docker.com/desktop/setup/install/windows-install/",
            },
            { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
          ],
        }
      }
      return {
        primary: {
          label: "Docker Desktop + WSL2",
          url: "https://docs.docker.com/desktop/setup/install/windows-install/",
        },
        alternatives: [
          { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
          {
            label: "Docker Engine on WSL",
            url: "https://docs.docker.com/engine/install/",
          },
        ],
      }
    case "macos":
      return {
        primary: { label: "Docker Desktop", url: "https://www.docker.com/products/docker-desktop/" },
        alternatives: [
          { label: "OrbStack", url: "https://orbstack.dev" },
          { label: "Colima", url: "https://github.com/abiosoft/colima" },
          { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
        ],
      }
    default:
      return {
        primary: { label: "Docker Desktop", url: "https://www.docker.com/products/docker-desktop/" },
        alternatives: [
          { label: "OrbStack", url: "https://orbstack.dev" },
          { label: "Colima", url: "https://github.com/abiosoft/colima" },
          { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
          { label: "Linux dockerd", url: "https://docs.docker.com/engine/install/" },
        ],
      }
  }
}
