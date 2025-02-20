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

locals {
  timestamp = formatdate("YYYYMMDD-hhmmss", timestamp())
  instance_name = "tee-4-instance-${local.timestamp}"
}

provider "google" {
  project = var.project_id
}

resource "google_compute_instance" "tdx_instance" {
  name         = local.instance_name
  machine_type = "c3-standard-4"
  zone         = var.zone

  boot_disk {
    auto_delete = true
    initialize_params {
      image = "projects/ubuntu-os-cloud/global/images/ubuntu-2204-jammy-v20250112"
      size  = 10
      type  = "hyperdisk-balanced"
    }
  }

  network_interface {
    network = "default"
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

  metadata = {
    confidential_compute_type = "TDX"
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