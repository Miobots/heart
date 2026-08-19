import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.substitutions import LaunchConfiguration, Command
from launch_ros.actions import Node
from launch_ros.parameter_descriptions import ParameterValue

# Sanitize Snap library contamination from LD_LIBRARY_PATH if running from Snap VS Code / terminal
if 'LD_LIBRARY_PATH' in os.environ:
    os.environ['LD_LIBRARY_PATH'] = ':'.join([
        p for p in os.environ['LD_LIBRARY_PATH'].split(':')
        if '/snap/' not in p
    ])

def generate_launch_description():
    pkg_mio_description = get_package_share_directory('mio_description')
    default_xacro_path = os.path.join(pkg_mio_description, 'urdf', 'mio.urdf.xacro')

    use_sim_time_arg = DeclareLaunchArgument(
        'use_sim_time',
        default_value='false',
        description='Use simulation (Gazebo) clock if true'
    )

    xacro_path_arg = DeclareLaunchArgument(
        'xacro_path',
        default_value=default_xacro_path,
        description='Absolute path to robot xacro file'
    )

    use_sim_time = LaunchConfiguration('use_sim_time')
    xacro_path = LaunchConfiguration('xacro_path')

    # Process Xacro to produce robot_description XML string (quoted to support spaces in path)
    robot_description = ParameterValue(
        Command(['xacro "', xacro_path, '"']),
        value_type=str
    )

    # Robot State Publisher Node
    node_robot_state_publisher = Node(
        package='robot_state_publisher',
        executable='robot_state_publisher',
        output='screen',
        parameters=[{
            'robot_description': robot_description,
            'use_sim_time': use_sim_time,
        }]
    )

    return LaunchDescription([
        use_sim_time_arg,
        xacro_path_arg,
        node_robot_state_publisher,
    ])
