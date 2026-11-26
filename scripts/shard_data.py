import os
import numpy as np
import pandas as pd

# 1. Establish robust cross-platform paths relative to the script location
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(SCRIPT_DIR)
output_dir = os.path.join(PROJECT_ROOT, "data")

# 2. Automatically locate the raw data file inside the data/ folder
possible_names = ["financial_dataset.csv", "Financial_Risk_Dataset.csv", "Financial Risk Dataset.csv"]
input_file = None

for name in possible_names:
    test_path = os.path.join(output_dir, name)
    if os.path.exists(test_path):
        input_file = test_path
        break

print(f"Target Shards Directory: {output_dir}")

if input_file is None:
    raise FileNotFoundError(
        f"CRITICAL ERROR: Could not find your source dataset inside: {output_dir}\n"
        f"Please verify your raw Kaggle CSV is placed in that folder and named 'financial_dataset.csv'"
    )

print(f"Found source dataset at: {input_file}")
print("Executing Dirichlet non-IID sharding across 8 nodes...")

# 3. Ingest the dataset and verify core default targets
df = pd.read_csv(input_file)
num_nodes = 8
alpha = 0.5  # Dirichlet concentration parameter for non-IID skew matching examiner rules

# Extract label column indices strictly (0 = Low Risk Profile, 1 = Default Risk Case)
class_0_idx = df[df["Loan Default Risk"] == 0].index.values
class_1_idx = df[df["Loan Default Risk"] == 1].index.values

# Seed random generation for strict examiner reproducibility checks
np.random.seed(42)
proportions_0 = np.random.dirichlet([alpha] * num_nodes)
proportions_1 = np.random.dirichlet([alpha] * num_nodes)

# Compute fractional matrix boundaries over proportions 
class_0_splits = np.split(class_0_idx, (np.cumsum(proportions_0)[:-1] * len(class_0_idx)).astype(int))
class_1_splits = np.split(class_1_idx, (np.cumsum(proportions_1)[:-1] * len(class_1_idx)).astype(int))

# 4. Generate isolated client profiles matrix slices
for i in range(num_nodes):
    node_idx = np.concatenate([class_0_splits[i], class_1_splits[i]])
    np.random.shuffle(node_idx)
    node_df = df.iloc[node_idx]
    
    # Save isolated client files matching your network volume configurations
    output_file_path = os.path.join(output_dir, f"client_{i+1}.csv")
    node_df.to_csv(output_file_path, index=False)
    print(f" -> client_{i+1}.csv generated with {len(node_df)} records. [High Risk Default Cases: {node_df['Loan Default Risk'].sum()}]")

print("Processing complete! Dataset has been partitioned successfully into 8 distinct shards.")
