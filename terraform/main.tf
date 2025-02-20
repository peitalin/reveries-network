terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 4.0"
    }
  }
}

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "eigen-413918"
}

variable "zone" {
  description = "GCP Zone"
  type        = string
  default     = "asia-southeast1-a"
}

variable "github_token" {
  description = "GitHub Personal Access Token"
  type        = string
  sensitive   = true
}

variable "github_repo" {
  description = "GitHub repository URL (format: owner/repo)"
  type        = string
}

variable "repo_branch" {
  description = "Repository branch to clone"
  type        = string
  default     = "main"
}

locals {
  timestamp = formatdate("YYYYMMDD-hhmmss", timestamp())
  instance_name = "tee-04-instance-${local.timestamp}"

  # Create startup script
  startup_script = <<-EOF
    #!/bin/bash
    set -euxo pipefail

    # Wait for cloud-init to complete
    cloud-init status --wait

    # Install system dependencies
    sudo apt-get install -y \
      apt-get install -y \
      git \
      pkg-config \
      libssl-dev \
      libtss2-dev \
      build-essential \
      curl

    # Install Rust
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"

    # Install Just
    curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sudo bash -s -- --to /usr/local/bin

    # Clone the repository using token in URL
    sudo mkdir -p /opt/app
    git clone https://${var.github_token}@github.com/${var.github_repo}.git
    sudo chown $(whoami):$(whoami) ~/1up-network
    cd ~/1up-network
    git checkout ${var.repo_branch}

    # Run the justfile command
    just node1
  EOF
}

provider "google" {
  project = var.project_id
}

# Static IP address
resource "google_compute_address" "static_ip" {
  name = "${local.instance_name}-ip"
  region = substr(var.zone, 0, length(var.zone)-2)
}

# GCP Confidential VM setup
# Instance Type: c3-standard-* family
# Operating System: containerOS, RHEL0, SLES-15-sp5, Ubuntu 22.04
# Supported Zones: asia-southeast-1-{a,b,c}, europe-west4-{a,b}, us-central1-{a,b,c}
#
# For more info on supported operating systems, check out GCP supported configurations:
# https://cloud.google.com/confidential-computing/confidential-vm/docs/supported-configurations#intel-tdx
# Currently, TDX enabled VMs can only be created via gcloud or Rest API:
# https://cloud.google.com/confidential-computing/confidential-vm/docs/create-a-confidential-vm-instance#gcloud

resource "google_compute_instance" "tdx_instance" {
  name         = local.instance_name
  machine_type = "c3-standard-4"
  zone         = var.zone

  boot_disk {
    auto_delete = true
    device_name = "boot-disk-${local.timestamp}"
    initialize_params {
      image = "projects/ubuntu-os-cloud/global/images/ubuntu-2204-jammy-v20250112"
      size  = 10
      type  = "hyperdisk-balanced"
    }
  }

  network_interface {
    network = "default"  # Using default VPC network
    access_config {
      nat_ip = google_compute_address.static_ip.address
    }
  }

  service_account {
    email  = "634774300751-compute@developer.gserviceaccount.com"
    scopes = ["cloud-platform"]
  }

  scheduling {
    on_host_maintenance = "TERMINATE"
  }

  shielded_instance_config {
    enable_secure_boot = false
    enable_vtpm = true
    enable_integrity_monitoring = true
  }

  // Specify TDX through advanced_machine_features instead
  advanced_machine_features {
    enable_nested_virtualization = false  // Default for TDX
    threads_per_core = null  // Default
    visible_core_count = null  // Default
  }

  confidential_instance_config {
    enable_confidential_compute = true
  }

  metadata = {
    startup-script = local.startup_script
  }

  tags = ["http-server", "https-server"]

  labels = {
    goog-ec-src = "vm_add-gcloud"
  }

  reservation_affinity {
    type = "ANY_RESERVATION"
  }

  deletion_protection = false
}